<script setup lang="ts">
/**
 * SteamQuestHelper
 *
 * Implements the community-verified workaround described in GitHub issue #48
 * for games whose Discord "detectable" entry no longer ships an `executables`
 * field:
 *
 *   1. The dummy executable is placed inside the Steam library folder:
 *      `<steamapps>/common/<install_dir>/<exe>.exe`
 *   2. A minimal `appmanifest_<app_id>.acf` file is written next to it in
 *      `<steamapps>/` so Steam/Discord consider the game "installed".
 *   3. Running the dummy exe from that folder makes Discord count play time
 *      towards the quest (Steam itself does not need to be running, but it
 *      must be installed).
 *
 * All file operations happen on the Rust side (steam.rs) which refuses to
 * touch folders or acf files that were not created by this app.
 */
import { computed, nextTick, ref, useTemplateRef, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { Game, SteamDummyStatus, SteamLibrary, SteamOverrideEntry } from '@/types/types';
import {
  findSteamOverride,
  getSteamSkus,
  sanitizeSegment,
} from '@/utils/steam-overrides';
import { useGlobalState } from '@/composables/app-state';
import { useDebounceFn } from '@vueuse/core';

const props = defineProps<{
  game: Game
}>();

const emit = defineEmits<{
  runningChange: [{ running: boolean, name: string | null }]
}>();

const { addLog } = useGlobalState();

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------
const libraries = ref<SteamLibrary[]>([]);
const librariesLoading = ref(true);
const selectedSteamapps = ref<string>('');
const manualSteamapps = ref<string>('');

const steamAppId = ref<string>('');
const installDir = ref<string>('');
const exeNamesText = ref<string>('');

const matchedOverride = ref<SteamOverrideEntry | undefined>(undefined);
const steamSkus = ref<string[]>([]);

const status = ref<SteamDummyStatus | null>(null);
const busy = ref(false);
const busyLabel = ref('');
const errorMsg = ref<string>('');
const infoMsg = ref<string>('');
const runningExes = ref<Record<string, boolean>>({});

const steamIssueUrl = 'https://github.com/markterence/discord-quest-completer/issues/48';
const steamDbUrl = computed(() =>
  steamAppId.value
    ? `https://steamdb.info/app/${steamAppId.value}/config/`
    : 'https://steamdb.info/'
);

const exeNames = computed(() =>
  exeNamesText.value
    .split(',')
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
);

const effectiveSteamapps = computed(() => {
  if (selectedSteamapps.value) return selectedSteamapps.value;
  return manualSteamapps.value.trim().replace(/\//g, '\\');
});

const formIssues = computed<string[]>(() => {
  const issues: string[] = [];
  if (!effectiveSteamapps.value) {
    issues.push('steamapps folder path is missing');
  }
  if (!/^\d+$/.test(steamAppId.value.trim())) {
    issues.push('Steam AppID is missing or not numeric');
  }
  if (!installDir.value.trim()) {
    issues.push('install folder name is missing');
  }
  if (!exeNames.value.length) {
    issues.push('at least one executable name is required');
  } else if (!exeNames.value.every((e) => e.toLowerCase().endsWith('.exe'))) {
    issues.push('every executable name must end with .exe');
  }
  return issues;
});

const formValid = computed(() => formIssues.value.length === 0);

const anyRunning = computed(() => Object.values(runningExes.value).some(Boolean));

// ---------------------------------------------------------------------------
// In-app confirm dialog (avoids native dialog plugin issues entirely)
// ---------------------------------------------------------------------------
const confirmDialogRef = useTemplateRef<HTMLDialogElement>('confirmDialogRef');
const confirmVisible = ref(false);
const confirmTitle = ref('');
const confirmText = ref('');
const confirmOkLabel = ref('OK');
let confirmResolver: ((value: boolean) => void) | null = null;

function askConfirm(text: string, options: { title: string, okLabel: string }): Promise<boolean> {
  confirmText.value = text;
  confirmTitle.value = options.title;
  confirmOkLabel.value = options.okLabel;
  confirmVisible.value = true;
  void nextTick(() => {
    confirmDialogRef.value?.showModal();
  });
  return new Promise((resolve) => {
    confirmResolver = resolve;
  });
}

function answerConfirm(value: boolean) {
  confirmDialogRef.value?.close();
  confirmVisible.value = false;
  confirmResolver?.(value);
  confirmResolver = null;
}

// ---------------------------------------------------------------------------
// Data loading
// ---------------------------------------------------------------------------
async function findLibraries() {
  librariesLoading.value = true;
  try {
    const result = await invoke<SteamLibrary[]>('find_steam_libraries');
    libraries.value = result ?? [];
    if (libraries.value.length > 0) {
      selectedSteamapps.value = libraries.value[0].steamapps;
      addLog(`Found ${libraries.value.length} Steam library folder(s).`);
    } else {
      addLog('warning', 'No Steam library folder detected automatically.');
    }
  } catch (e) {
    addLog('error', 'Failed to locate Steam libraries: ' + String(e));
    libraries.value = [];
  } finally {
    librariesLoading.value = false;
  }
}

function prefillFromGame(game: Game) {
  matchedOverride.value = findSteamOverride(game);
  steamSkus.value = getSteamSkus(game);

  if (matchedOverride.value) {
    steamAppId.value = matchedOverride.value.steam_app_id;
    installDir.value = matchedOverride.value.install_dir;
    exeNamesText.value = matchedOverride.value.exe_names.join(', ');
    addLog(`Using bundled Steam data for '${game.name}'.`);
  } else {
    steamAppId.value = steamSkus.value[0] ?? '';
    const base = sanitizeSegment(game.name ?? '');
    installDir.value = base;
    exeNamesText.value = base ? `${base}.exe` : '';
    if (!steamAppId.value) {
      addLog('warning', `'${game.name}' has no Steam AppID listed by Discord.`);
    } else {
      addLog(
        'warning',
        `No bundled data for '${game.name}'. Check SteamDB for the correct install dir and exe name.`,
      );
    }
  }
}

const refreshStatus = useDebounceFn(async () => {
  if (!effectiveSteamapps.value || !/^\d+$/.test(steamAppId.value.trim()) || !installDir.value.trim()) {
    status.value = null;
    return;
  }
  try {
    status.value = await invoke<SteamDummyStatus>('get_steam_dummy_status', {
      steamapps: effectiveSteamapps.value,
      app_id: Number(steamAppId.value.trim()),
      install_dir: installDir.value.trim(),
    });
  } catch (e) {
    addLog('error', 'Status check failed: ' + String(e));
    status.value = null;
  }
}, 350);

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------
async function install() {
  errorMsg.value = '';
  infoMsg.value = '';
  if (!formValid.value || busy.value) return;

  let confirmed = false;
  try {
    confirmed = await askConfirm(
      [
        'This will write files into your Steam library folder:',
        '',
        `  ${effectiveSteamapps.value}\\common\\${installDir.value.trim()}\\`,
        `  ${effectiveSteamapps.value}\\appmanifest_${steamAppId.value.trim()}.acf`,
        '',
        'Files or folders that were not created by this app are never overwritten.',
        '',
        'Recommended: in Steam settings, disable automatic game updates while the dummy is installed (see issue #48).',
        '',
        'Continue?',
      ].join('\n'),
      { title: 'Install dummy game into Steam library', okLabel: 'Install' },
    );
  } catch (e) {
    addLog('error', 'Confirmation dialog failed: ' + String(e));
    return;
  }
  if (!confirmed) {
    addLog('Install cancelled by user.');
    return;
  }

  busy.value = true;
  busyLabel.value = 'Installing...';
  try {
    addLog(`Installing Steam dummy for '${props.game.name}' (appid ${steamAppId.value.trim()})...`);
    const result = await invoke<SteamDummyStatus>('install_steam_dummy_game', {
      steamapps: effectiveSteamapps.value,
      app_id: Number(steamAppId.value.trim()),
      game_name: props.game.name,
      install_dir: installDir.value.trim(),
      exe_names: exeNames.value,
    });
    status.value = result;
    infoMsg.value = 'Dummy game installed into the Steam library.';
    addLog(`Installed Steam dummy in ${result.folder}`);
  } catch (e) {
    errorMsg.value = String(e);
    addLog('error', `Steam install failed: ${String(e)}`);
  } finally {
    busy.value = false;
    busyLabel.value = '';
  }
}

async function play(exe: string) {
  errorMsg.value = '';
  busy.value = true;
  busyLabel.value = 'Starting...';
  try {
    await invoke('run_steam_dummy_game', {
      steamapps: effectiveSteamapps.value,
      install_dir: installDir.value.trim(),
      exe_name: exe,
      title: props.game.name,
    });
    runningExes.value[exe] = true;
    infoMsg.value = `Started ${exe}. Watch Discord - it should soon show that you are playing ${props.game.name}.`;
    emit('runningChange', { running: true, name: props.game.name });
    addLog(`Started Steam dummy process: ${exe}`);
  } catch (e) {
    errorMsg.value = String(e);
    addLog('error', `Failed to start ${exe}: ${String(e)}`);
  } finally {
    busy.value = false;
    busyLabel.value = '';
  }
}

async function stop(exe: string) {
  errorMsg.value = '';
  busy.value = true;
  busyLabel.value = 'Stopping...';
  try {
    await invoke('stop_process', { exec_name: exe });
    addLog(`Stopped process: ${exe}`);
  } catch (e) {
    // The process may simply not exist anymore; that is fine.
    addLog('warning', `Stop failed for ${exe}: ${String(e)}`);
  } finally {
    runningExes.value[exe] = false;
    if (!anyRunning.value) {
      emit('runningChange', { running: false, name: null });
    }
    busy.value = false;
    busyLabel.value = '';
  }
}

async function uninstall() {
  errorMsg.value = '';
  infoMsg.value = '';
  if (busy.value) return;

  const confirmed = await askConfirm(
    'Remove the dummy executable(s) and the appmanifest .acf file created by this app? Real game files are never touched.',
    { title: 'Remove dummy game', okLabel: 'Remove' },
  );
  if (!confirmed) return;

  busy.value = true;
  busyLabel.value = 'Removing...';
  try {
    const result = await invoke<string>('remove_steam_dummy_game', {
      steamapps: effectiveSteamapps.value,
      app_id: Number(steamAppId.value.trim()),
      install_dir: installDir.value.trim(),
    });
    addLog(`Removed Steam dummy for '${props.game.name}': ${result}`);
    infoMsg.value = result;
    Object.keys(runningExes.value).forEach((k) => (runningExes.value[k] = false));
    emit('runningChange', { running: false, name: null });
    await refreshStatus();
  } catch (e) {
    errorMsg.value = String(e);
    addLog('error', `Remove failed: ${String(e)}`);
  } finally {
    busy.value = false;
    busyLabel.value = '';
  }
}

// ---------------------------------------------------------------------------
// Watchers
// ---------------------------------------------------------------------------
watch(
  () => props.game,
  (game) => {
    runningExes.value = {};
    prefillFromGame(game);
    refreshStatus();
  },
  { immediate: true },
);

watch([effectiveSteamapps, steamAppId, installDir], () => {
  refreshStatus();
});

findLibraries();
</script>

<template>
  <div class="text-sm text-gray-500 dark:text-gray-400">
    <!-- In-app confirm dialog -->
    <dialog v-if="confirmVisible" ref="confirmDialogRef" class="dialogStyle inset-0 bg-gray-800 bg-opacity-50
        border border-gray-300 dark:border-gray-600 rounded-lg
        transition-opacity duration-300 ease-in-out z-50"
        style="left: 50%; top: 50%; transform: translate(-50%, -50%)"
        @cancel.prevent="answerConfirm(false)">
      <div class="flex flex-col items-center justify-center p-6 max-w-md">
        <h3 class="text-base font-semibold text-gray-800 dark:text-white mb-3">{{ confirmTitle }}</h3>
        <div class="mb-4 text-gray-500 dark:text-gray-400 whitespace-pre-line text-sm">{{ confirmText }}</div>
        <div class="gap-2 flex">
          <button
            class="text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-300
              border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-1"
            @click="answerConfirm(false)">
            Cancel
          </button>
          <button
            class="text-white bg-indigo-600 hover:bg-indigo-700 rounded-lg px-4 py-1"
            autofocus
            @click="answerConfirm(true)">
            {{ confirmOkLabel }}
          </button>
        </div>
      </div>
    </dialog>

    <!-- Why this exists -->
    <div class="p-3 mb-3 border border-amber-400/50 bg-amber-500/10 rounded-lg text-xs">
      <p class="font-semibold text-amber-600 dark:text-amber-400 mb-1">
        This game has no executable data from Discord.
      </p>
      <p>
        Discord changed its quest detection for newer games (see
        <a :href="steamIssueUrl" target="_blank" class="underline text-indigo-500 dark:text-indigo-400">issue #48</a>).
        The workaround needs <strong>Steam</strong>: a dummy executable is placed in your Steam library
        together with a small <span class="font-mono">appmanifest_*.acf</span> file so the game looks installed.
      </p>
    </div>

    <!-- Steam library selection -->
    <div class="mb-3">
      <label class="block text-xs font-semibold mb-1">Steam library folder (steamapps)</label>
      <select v-if="libraries.length > 0" v-model="selectedSteamapps"
        class="w-full px-2 py-1.5 border border-gray-300 dark:border-gray-600 rounded-md dark:bg-gray-700 dark:text-white text-xs">
        <option v-for="lib in libraries" :key="lib.steamapps" :value="lib.steamapps">
          {{ lib.steamapps }}
        </option>
      </select>
      <div v-else>
        <p v-if="librariesLoading" class="text-xs">Detecting Steam installation...</p>
        <div v-else>
          <p class="text-xs text-amber-600 dark:text-amber-400 mb-1">
            Steam was not detected automatically. Paste the full path of your
            <span class="font-mono">steamapps</span> folder below.
          </p>
          <input v-model="manualSteamapps" type="text" spellcheck="false"
            placeholder="C:\Program Files (x86)\Steam\steamapps"
            class="w-full px-2 py-1.5 border border-gray-300 dark:border-gray-600 rounded-md dark:bg-gray-700 dark:text-white text-xs font-mono" />
        </div>
      </div>
    </div>

    <!-- Steam metadata form -->
    <div class="grid grid-cols-2 gap-2 mb-2">
      <div>
        <label class="block text-xs font-semibold mb-1">Steam AppID</label>
        <input v-model="steamAppId" type="text" inputmode="numeric" spellcheck="false"
          placeholder="e.g. 3065800"
          class="w-full px-2 py-1.5 border border-gray-300 dark:border-gray-600 rounded-md dark:bg-gray-700 dark:text-white text-xs font-mono" />
      </div>
      <div>
        <label class="block text-xs font-semibold mb-1">Install folder name</label>
        <input v-model="installDir" type="text" spellcheck="false" placeholder="e.g. Marathon"
          class="w-full px-2 py-1.5 border border-gray-300 dark:border-gray-600 rounded-md dark:bg-gray-700 dark:text-white text-xs font-mono" />
      </div>
    </div>
    <div class="mb-2">
      <label class="block text-xs font-semibold mb-1">Executable name(s) (comma separated)</label>
      <input v-model="exeNamesText" type="text" spellcheck="false" placeholder="e.g. Marathon.exe, MarathonLauncher.exe"
        class="w-full px-2 py-1.5 border border-gray-300 dark:border-gray-600 rounded-md dark:bg-gray-700 dark:text-white text-xs font-mono" />
    </div>

    <p class="text-xs mb-3" v-if="matchedOverride">
      ✓ Using bundled data for this game ({{ matchedOverride.name }}).
    </p>
    <p class="text-xs mb-3" v-else>
      No bundled data. Find the correct values on
      <a :href="steamDbUrl" target="_blank" class="underline text-indigo-500 dark:text-indigo-400">SteamDB</a>:
      look for <em>installdir</em> and the default launch option's <em>Executable</em>.
    </p>

    <!-- Status -->
    <div v-if="status" class="text-xs mb-3 p-2 rounded-md border"
      :class="status.installed ? 'border-green-500/50 bg-green-500/10' : 'border-gray-300 dark:border-gray-600'">
      <template v-if="status.installed">
        <span class="text-green-600 dark:text-green-400 font-semibold">Dummy installed</span>
        in {{ status.folder }}
      </template>
      <template v-else-if="status.acf_exists || status.marker_exists">
        Partial install detected (acf: {{ status.acf_exists ? 'yes' : 'no' }}, folder files: {{ status.marker_exists ? 'yes' : 'no' }}).
        Remove leftovers before reinstalling.
      </template>
      <template v-else>Not installed yet.</template>
    </div>

    <!-- Errors / info -->
    <div v-if="errorMsg" class="text-xs mb-3 p-2 rounded-md border border-red-500/50 bg-red-500/10 text-red-600 dark:text-red-400 whitespace-pre-wrap">
      {{ errorMsg }}
    </div>
    <div v-if="infoMsg" class="text-xs mb-3 p-2 rounded-md border border-green-500/50 bg-green-500/10 text-green-600 dark:text-green-400">
      {{ infoMsg }}
    </div>

    <!-- Actions -->
    <button v-if="!status?.installed" :disabled="!formValid || busy" @click="install"
      class="w-full py-2 rounded-lg mb-2 text-white"
      :class="(!formValid || busy) ? 'bg-indigo-400 cursor-not-allowed' : 'bg-indigo-600 hover:bg-indigo-700'">
      {{ busy ? busyLabel : 'Install dummy in Steam library' }}
    </button>

    <!-- What is missing before install can run -->
    <ul v-if="!status?.installed && !formValid" class="text-xs mb-3 list-disc list-inside text-amber-600 dark:text-amber-400">
      <li v-for="issue in formIssues" :key="issue">{{ issue }}</li>
    </ul>

    <template v-else>
      <div v-for="exe in status?.exes ?? []" :key="exe" class="flex items-center gap-2 mb-2">
        <span class="flex-1 font-mono text-xs truncate">{{ exe }}</span>
        <button :disabled="busy" @click="runningExes[exe] ? stop(exe) : play(exe)"
          class="px-3 py-1 rounded-md text-white text-xs"
          :class="runningExes[exe] ? 'bg-red-500 hover:bg-red-600' : 'bg-green-600 hover:bg-green-700'">
          {{ runningExes[exe] ? 'Stop' : 'Play' }}
        </button>
      </div>
      <button :disabled="busy || anyRunning" @click="uninstall"
        class="w-full py-2 rounded-lg text-white mt-1"
        :class="(busy || anyRunning) ? 'bg-gray-400 cursor-not-allowed' : 'bg-red-600 hover:bg-red-700'">
        {{ busy ? busyLabel : 'Remove dummy from Steam library' }}
      </button>
    </template>

    <!-- Footnotes -->
    <div class="text-xs mt-3 border-t border-gray-200 dark:border-gray-700 pt-2 space-y-1">
      <p>• If Steam starts "updating" the fake game, cancel it and disable auto-updates in Steam settings.</p>
      <p>• You may close Steam afterwards; Discord counts quest progress while the dummy exe runs.</p>
      <p>• Keep the game "running" for the whole quest duration (usually 15 minutes).</p>
    </div>
  </div>
</template>

<style scoped>
@reference "../theme/style.css";

.dialogStyle::backdrop {
  @apply bg-black/70 backdrop-blur-xs;
}
</style>

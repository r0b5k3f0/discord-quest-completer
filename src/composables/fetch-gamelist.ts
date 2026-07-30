import { Game } from '@/types/types';
import { tryOnMounted, tryOnScopeDispose, useAsyncState } from '@vueuse/core';
import { ref, shallowRef } from 'vue';
import { message } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { useGlobalState } from './app-state';

/** How long the "done" toasts stay on screen after a fetch finishes. */
const DONE_DELAY_MS = 1800;

type GameListSource = 'github_mirror' | 'discord';

export function useFetchGameList() {
    const { addLog } = useGlobalState();

    function fetchRemote(source: GameListSource) {
        return () => invoke<Game[]>('fetch_gamelist', { source });
    }

    const {
        state: gameListGHMirror,
        error: errorGH,
        isReady: isReadyGH,
        execute: executeGH,
        isLoading: isLoadingGH
    } = useAsyncState<Game[]>(fetchRemote('github_mirror'), [], {
        immediate: false,
        resetOnExecute: true,
    });

    const {
        state: gameListFromDiscord,
        error: errorDiscord,
        isReady: isReadyDiscord,
        execute: executeDiscord,
        isLoading: isLoadingDiscord
    } = useAsyncState<Game[]>(fetchRemote('discord'), [], {
        immediate: false,
        resetOnExecute: true,
    });

    const {
        state: bundledGameList,
        error: errorBundled,
        isReady: isReadyBundled,
        execute: executeBundled,
        isLoading: isLoadingBundled
    } = useAsyncState<Game[]>(
        () => import('@/assets/gamelist.json').then((res) => res.default),
        [],
        {
            immediate: false,
            resetOnExecute: true,
        },
    );

    const fetchError = ref<string | null>(null);

    // Shallow on purpose: this holds ~24k entries that are only ever replaced
    // wholesale. A deep ref would make Vue wrap every game (and its nested
    // arrays) in a Proxy, which Fuse then pays for again while indexing.
    const gameDB = shallowRef<Game[]>([]);

    const allFetchDone = ref(false);

    function isValidGameList(data: unknown): data is Game[] {
        return (
            Array.isArray(data) &&
            data.length > 0 &&
            !!data[0] &&
            'aliases' in data[0] &&
            'name' in data[0] &&
            'executables' in data[0]
        );
    }

    let doneTimeout: ReturnType<typeof setTimeout> | null = null;

    function clearDoneTimeout() {
        if (doneTimeout !== null) {
            clearTimeout(doneTimeout);
            doneTimeout = null;
        }
    }

    // Leaves the "fetched ✓" toasts up briefly before the container hides.
    function scheduleDone() {
        clearDoneTimeout();
        doneTimeout = setTimeout(() => {
            allFetchDone.value = true;
            doneTimeout = null;
        }, DONE_DELAY_MS);
    }

    tryOnScopeDispose(clearDoneTimeout);

    async function fetchGameList() {
        allFetchDone.value = false;
        clearDoneTimeout();
        fetchError.value = null;
        addLog('Fetching game list...');

        // 1. GitHub mirror first: it is a static file on a CDN.
        addLog('Fetching game list from GitHub mirror...');
        await executeGH();
        if (!errorGH.value && isValidGameList(gameListGHMirror.value)) {
            gameDB.value = gameListGHMirror.value;
            addLog(`Using game list from GitHub mirror. ${gameDB.value.length} entries.`);
            scheduleDone();
            return;
        }
        addLog('error', `Could not use the game list from the GitHub mirror. ${errorGH.value ?? ''}`);

        // 2. Straight from Discord.
        addLog('Fetching game list directly from Discord...');
        await executeDiscord();
        if (!errorDiscord.value && isValidGameList(gameListFromDiscord.value)) {
            gameDB.value = gameListFromDiscord.value;
            addLog(`Using game list from Discord. ${gameDB.value.length} entries.`);
            scheduleDone();
            return;
        }
        addLog('error', `Could not use the game list from Discord. ${errorDiscord.value ?? ''}`);

        // 3. Bundled copy. Deliberately loaded only at this point: it is a
        //    ~12 MB asset that used to be parsed on every start just to be
        //    thrown away whenever the network worked.
        addLog('Loading the bundled game list as a fallback...');
        await executeBundled();
        if (!errorBundled.value && isValidGameList(bundledGameList.value)) {
            gameDB.value = bundledGameList.value;
            fetchError.value =
                'Could not reach the online game lists, so the bundled copy is being used. It may be out of date.';
            addLog('warning', `Using bundled game list as fallback. ${gameDB.value.length} entries.`);
        } else {
            fetchError.value = 'Could not load any game list.';
            addLog('error', `Could not load the bundled game list either. ${errorBundled.value ?? ''}`);
        }

        await message(fetchError.value, {
            title: 'Game List Fetch Error',
            kind: 'error',
            buttons: {
                ok: 'OK'
            }
        });

        scheduleDone();
    }

    tryOnMounted(async () => {
        await fetchGameList();
    });

    return {
        gameListGHMirror,
        gameListFromDiscord,
        bundledGameList,
        fetchError,
        isReadyGH,
        isReadyDiscord,
        isReadyBundled,
        gameDB,
        fetchGameList,
        isLoadingGH,
        isLoadingDiscord,
        isLoadingBundled,
        allFetchDone
    }
}

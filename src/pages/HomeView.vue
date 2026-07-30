<script setup lang="ts">
import { ref, computed, useTemplateRef, shallowRef, provide, onUnmounted } from 'vue';
import { onClickOutside, refDebounced } from '@vueuse/core';
import { useFuse } from '@vueuse/integrations/useFuse'
import { invoke } from '@tauri-apps/api/core';
import { randomString } from '@/utils/random-string';
import { GameActionsProvider, GameExecutable, type Game } from '@/types/types';
import IconVerified from '@/components/IconVerified.vue';
import GameExecutables from '@/components/GameExecutables.vue';
import SteamQuestHelper from '@/components/SteamQuestHelper.vue';
import { gameHasWindowsExecutables } from '@/utils/steam-overrides';
import { GameActionsKey } from '@/constants/constants';
import { emit, listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useFetchGameList } from '@/composables/fetch-gamelist';
import { UseFuseOptions } from '@vueuse/integrations';
import Fuse from 'fuse.js';
import { useGlobalState } from '@/composables/app-state';
import TimedNotification from '@/components/TimedNotification.vue';


type DialogKey =
    'none' |
    'rpc_message_1'|
    'no_game_selected';

const {
    gameDB,
    isLoadingBundled,
    isLoadingDiscord,
    isLoadingGH,
    fetchGameList,
    isReadyGH,
    isReadyBundled,
    isReadyDiscord,
    allFetchDone,
} = useFetchGameList()
const { addLog } = useGlobalState();
const shouldShowNotificationContainer = computed(() => {
    return isLoadingGH.value || isLoadingDiscord.value || isLoadingBundled.value ||
           (isReadyGH.value || isReadyDiscord.value || isReadyBundled.value);
});

const dialogRef = useTemplateRef<HTMLDialogElement>('dialogRef');
const searchResultContainerRef = useTemplateRef<HTMLElement>('searchResultContainerRef')
const dialogKey = ref<DialogKey>('none')
const isConnectedToRPC = ref(false);
const isConnecting = ref(false);

// Search functionality
const searchQuery = shallowRef('');
const debouncedSearchQuery = refDebounced(searchQuery, 300)

const searchResultsIsOpen = ref(false);

// Game status
const currentlyPlaying = ref<string | null>(null);


onClickOutside(searchResultContainerRef, () => {
    searchResultsIsOpen.value = false;
})

const COPYRIGHT_SYMBOL = '\u00A9';
const TRADEMARK_SYMBOL = '\u2122';
const REGISTERED_SYMBOL = '\u00AE';
const ignoredSymbols = [COPYRIGHT_SYMBOL, TRADEMARK_SYMBOL, REGISTERED_SYMBOL];
const ignoredSymbolsRegex = new RegExp(`[${ignoredSymbols.join('')}]`, 'g');
const fuseOptions = computed<UseFuseOptions<Game>>(() => ({
    fuseOptions: {
        // Prioritize name and aliases for searching, then lastly executables
        keys: [
            { name: 'name', weight: 0.7 },
            { name: 'aliases', weight: 0.2 },
            { name: 'executables.name', weight: 0.1 },
        ],
        getFn: (obj: any, path: string[] | string) => {
            const value = Fuse.config.getFn(obj, path);
            return typeof value === "string"
            ? value.replace(ignoredSymbolsRegex, "")
            : value;
        },
        isCaseSensitive: false,
        threshold: 0.5,        
        // A score of 0indicates a perfect match, while a score of 1 indicates a complete mismatch
        includeScore: true,
        includeMatches: false
    },
    resultLimit: 12,
    matchAllWhenSearchEmpty: false,
}));

const { results: searchResults } = useFuse(debouncedSearchQuery, gameDB, fuseOptions)

// Selected games list
const gameList = ref<Game[]>([]);
const selectedGameId = ref<string | null | undefined>(null);

const selectedGame = computed(() => {
    if (!selectedGameId.value) return null;
    return gameList.value.find(g => g.uid === selectedGameId.value) || null;
});

// Discord no longer ships the `executables` field for many new games (issue #48).
// Those need the Steam workaround instead of the plain dummy-exe flow.
const selectedGameHasWinExes = computed(() => gameHasWindowsExecutables(selectedGame.value));

function handleSteamRunningChange({ running }: { running: boolean, name: string | null }) {
    if (selectedGame.value) {
        selectedGame.value.is_running = running;
    }
    currentlyPlaying.value = running ? (selectedGame.value?.id ?? null) : null;
}

function closeSearchResults() {
    searchResultsIsOpen.value = false;
}
function openSearchResults() {
    searchResultsIsOpen.value = true;
}

// Function to add a game to the selected list
function addGameToList(game: Game) {
    if (!gameList.value.some(g => g.id === game.id)) {
        gameList.value.push({
            ...game,
            uid: randomString(),
            // The entries themselves come straight out of the shared game list,
            // so the executables have to be copied before this view starts
            // writing is_running / is_installed onto them.
            executables: game.executables.map(exe => ({ ...exe })),
        });
    }

    closeSearchResults();
}

// Function to remove a game from the selected list
function removeGameFromList(game: Game) {
    const gameId = game.uid;
    gameList.value = gameList.value.filter(game => game.uid !== gameId);
    if (selectedGameId.value === gameId) {
        selectedGameId.value = null;
    }
}

function selectGame(game: Game) {
    selectedGameId.value = game?.uid;
    searchResultsIsOpen.value = false;
}

function canPlayGame(game: Game | null) {
    if (!game) {
        return false;
    }
    // we can only play a game if the game is installed and not running
    return (game.is_installed && !game.is_running) ?? false;
}

function isExecutableRunning(executable: GameExecutable) {
    // Check if the executable is running
    return executable.is_running ?? false;
}
function isGameExecutableInstalled(executable: GameExecutable) {
    // Check if the executable is installed
    return executable.is_installed ?? false;
}

function isGameInstalled(game: Game | null) {
    if (!game) {
        return false;
    }
    // we can only play a game if the game is installed and not running
    return game.is_installed ?? false;
}


// Create a dummy game
async function createDummyGame(game: Game | null, executable: GameExecutable) {
    if (!game) {
        return;
    }
    const gameUid = game.uid;
    const gameToInstall = gameList.value.find(g => g.uid === gameUid);
    const executableItem = gameToInstall?.executables.find(exe => exe.name === executable.name);
    if (gameToInstall && executableItem) {
        try {
            await invoke('create_fake_game', {
                path: executable.path,
                executable_name: executable.filename,
                app_id: Number(gameToInstall.id),
            });
        } catch (error) {
            addLog('error', `Failed to create dummy game: ${String(error)}`);
            return false;
        }
        gameToInstall.is_installed = true;
        executableItem.is_installed = true;
        return true;
    }
    return false;
}


async function installAndPlay({game, executable}: {game: Game, executable: GameExecutable}) {
    if (!game) {
        return;
    }
    const gameCreated = await createDummyGame(game, executable);
    if (gameCreated) {
        await playGame({game, executable});
    } else {
        addLog('error', `Failed to create the dummy game for ${game.name}`);
    }
}
// Play game function
async function playGame({game, executable}: {game: Game, executable: GameExecutable}) {
    if (!game) {
        return;
    }
    const gameUid = game.uid;
    try {
        addLog('info', `Playing game: ${game.name}`);
        addLog('info', `Executable: ${executable.name}`);
        currentlyPlaying.value = game.id;
        // find the game in the list
        const gameToPlay = gameList.value.find(g => g.uid === gameUid);
        const executableItem = gameToPlay?.executables.find(exe => exe.name === executable.name);
        if (gameToPlay && executableItem) {
            await invoke('run_background_process', {
                name: game.name,
                path: executable.path,
                executable_name: executable.filename,
                app_id: Number(gameToPlay.id),
            });
            gameToPlay.is_running = true;
            executableItem.is_running = true;
        }
    } catch (error) {
        currentlyPlaying.value = null;
        addLog('error', `Failed to launch game: ${String(error)}`);
    }
}

// Stop playing
async function stopPlaying({game, executable}: {game: Game, executable: GameExecutable}) {
    if (!game) {
        return;
    }
    const gameUid = game.uid;

    currentlyPlaying.value = null;

    const gameToPlay = gameList.value.find(g => g.uid === gameUid);
    const executableItem = gameToPlay?.executables.find(exe => exe.name === executable.name);
    if (gameToPlay && executableItem) {
        try {
            await invoke('stop_process', {
                exec_name: executable.filename!
            })
            addLog('info', `Stopped game process: ${game.name}`);
            addLog('info', `Stopped Executable: ${executable.name}`);
        } catch (error) {
            const errorMessage = (error instanceof Error) ? error.message : String(error);
            addLog('error', 'Failed to stop game process: ' + errorMessage);
        } finally {
            // Even if stopping failed, the process is no longer ours to track.
            gameToPlay.is_running = false;
            executableItem.is_running = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Discord RPC
//
// `connect_to_discord_rpc_3` only schedules the connection attempt, so the
// actual outcome arrives as an event. Flipping the UI to "connected" straight
// after the invoke resolved used to claim success even when Discord was closed.
// ---------------------------------------------------------------------------
const rpcAppId = ref<string | null>(null);
const unlistenFns: UnlistenFn[] = [];
let disposed = false;

// `listen` resolves asynchronously, so a subscription can land after unmount.
function trackUnlisten(un: UnlistenFn) {
    if (disposed) un();
    else unlistenFns.push(un);
}

listen<{ app_id: string }>('client_connected', ({ payload }) => {
    isConnecting.value = false;
    isConnectedToRPC.value = true;
    rpcAppId.value = payload?.app_id ?? null;
    const game = gameList.value.find(g => g.id === rpcAppId.value);
    if (game) {
        game.is_running = true;
        currentlyPlaying.value = game.id;
    }
    addLog('info', 'Connected to Discord Gateway.');
}).then(trackUnlisten);

listen<{ message: string }>('client_error', ({ payload }) => {
    isConnecting.value = false;
    isConnectedToRPC.value = false;
    resetRPCGameState();
    addLog('error', `Discord RPC failed: ${payload?.message ?? 'unknown error'}`);
}).then(trackUnlisten);

onUnmounted(() => {
    disposed = true;
    unlistenFns.forEach(un => un());
    unlistenFns.length = 0;
});

function resetRPCGameState() {
    const game = gameList.value.find(g => g.id === rpcAppId.value);
    if (game) {
        game.is_running = false;
    }
    if (currentlyPlaying.value === rpcAppId.value) {
        currentlyPlaying.value = null;
    }
    rpcAppId.value = null;
}

async function handleTestRPC(game: Game | null) {
    if (isConnectedToRPC.value || isConnecting.value) {
        emit('event_disconnect');
        isConnectedToRPC.value = false;
        isConnecting.value = false;
        resetRPCGameState();
        addLog('info', 'Disconnected from Discord Gateway.');
        return;
    }
    if (!game) {
        showDialog('no_game_selected');
        return;
    }
    showDialog('rpc_message_1');
}

async function continueRPCRisk(game: Game | null) {
    if (!game) {
        return;
    }
    const gameToTest = gameList.value.find(g => g.uid === game.uid);
    if (!gameToTest) {
        return;
    }

    hideDialog();
    isConnecting.value = true;
    try {
        await invoke('connect_to_discord_rpc_3', {
            activity_json: JSON.stringify({
                app_id: gameToTest.id,
            }),
            action: 'connect',
        });
    } catch (error) {
        isConnecting.value = false;
        addLog('error', `Could not start the Discord connection: ${String(error)}`);
    }
}

function showDialog(message: DialogKey) {
    dialogKey.value = message;
    if (message !== 'none') {
        dialogRef.value?.showModal();
    }
}

function hideDialog() {
    dialogRef.value?.close();
    dialogKey.value = 'none';
}


provide<GameActionsProvider>(GameActionsKey, {
    canPlayGame,
    isGameInstalled,
    isExecutableRunning,
    isGameExecutableInstalled,
});
</script>

<template>
    <div class="container mx-auto px-4 py-8">
        <!-- Center dialog -->
        <dialog id="dialog" class="dialogStyle inset-0 bg-gray-800 bg-opacity-50
        border border-gray-300 dark:border-gray-600 rounded-lg
        transition-opacity duration-300 ease-in-out z-50
        "
        style="left: 50%; top: 50%; transform: translate(-50%, -50%)"
        ref="dialogRef">
            <div class="flex flex-col items-center justify-center p-6" >
                <div class="mb-4 text-gray-500 dark:text-gray-400">
                    <div v-if="dialogKey === 'rpc_message_1'">
                        <p>
                        This is only a feature in development.  
                        </p>
                        <p class="my-2">
                            It works but due to the nature that it tricks Discord into thinking you are playing a game
                            by sending an RPC using actual game ID rather than letting Discord detect you have a game/application running. 
                        </p>
                        <p>
                        This may flag your account as suspicious for self-botting.
                        </p>
                    </div>

                    <div v-if="dialogKey === 'no_game_selected'">
                        <p>
                            No game selected. Please select a game from the list on the left.
                        </p>
                    </div>
                </div>
                <div class="gap-2 flex">
                    <button
                    
                    class="
                text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-300 
                border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-1"
                @click="hideDialog()">
                    <span  v-if="dialogKey == 'rpc_message_1'">
                        Cancel 
                    </span>
                    <span v-else>OK</span>
                </button>
                
                <button 
                v-if="dialogKey === 'rpc_message_1'"
                class="text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-300 
                border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-1"
                @click="continueRPCRisk(selectedGame)">
                    Accept risk and continue
                </button>
                </div>
            </div>
        </dialog>
        <h1 class="text-3xl font-bold text-gray-900 dark:text-white mb-6 text-center">
            Handler
        </h1>

        <!-- refetch game list fetch status. will appear on top left -->
        <Transition 
            enter-active-class="transition-opacity duration-300 delay-100 ease-in-out"
            leave-active-class="transition-opacity duration-600 delay-100 ease-in-out"  
            enter-from-class="opacity-0 translate-y-2 ease-in-out"
            enter-to-class="opacity-100 translate-y-0 ease-in-out"
        >
            <div class="absolute top-20 left-4 z-20 " v-if="shouldShowNotificationContainer && !allFetchDone">
                <!-- Fetching from mirror loading indicator --> 
                <Transition 
                    enter-active-class="transition-opacity duration-300 delay-100 ease-in-out"
                    leave-active-class="transition-opacity duration-600 delay-100 ease-in-out"  
                    enter-from-class="opacity-0 translate-y-2 ease-in-out"
                    enter-to-class="opacity-100 translate-y-0 ease-in-out"
                >
                    <div v-if="isLoadingGH" class="text-sm text-gray-500 dark:text-gray-400">
                        Fetching game list from GitHub mirror... 
                      <div class="border-full h-2 w-2 bg-green-500 rounded-full inline-block ml-2 animate-pulse"></div>
                    </div>
                </Transition>
                <TimedNotification
                    :is-ready="isReadyGH" 
                    :duration="1500"
                    container-class="text-sm text-gray-500 dark:text-gray-400"
                > 
                    Game list from mirror fetched <span class="text-green-400">✓</span>
                </TimedNotification>

                <!-- Fetching from Discord loading indicator -->
                <Transition 
                    enter-active-class="transition-opacity duration-300 delay-100 ease-in-out"
                    leave-active-class="transition-opacity duration-600 delay-100 ease-in-out"  
                    enter-from-class="opacity-0 translate-y-2 ease-in-out"
                    enter-to-class="opacity-100 translate-y-0 ease-in-out"
                >
                    <div v-if="isLoadingDiscord" class="text-sm text-gray-500 dark:text-gray-400">
                        Fetching game list directly from Discord...
                        <div class="border-full h-2 w-2 bg-green-500 rounded-full inline-block ml-2 animate-pulse"></div>
                    </div>
                </Transition>
                <TimedNotification
                    :is-ready="isReadyDiscord" 
                    :duration="1500"
                    container-class="text-sm text-gray-500 dark:text-gray-400"
                > 
                    Game list from Discord fetched <span class="text-green-400">✓</span>
                </TimedNotification>

                
                <!-- Fetching from bundled loading indicator -->
                <Transition 
                    enter-active-class="transition-opacity duration-300 delay-100 ease-in-out"
                    leave-active-class="transition-opacity duration-600 delay-100 ease-in-out"  
                    enter-from-class="opacity-0 translate-y-2 ease-in-out"
                    enter-to-class="opacity-100 translate-y-0 ease-in-out"
                >
                    <div v-if="isLoadingBundled" class="text-sm text-gray-500 dark:text-gray-400">
                        Fetching game list from bundled game list...
                        <div class="border-full h-2 w-2 bg-green-500 rounded-full inline-block ml-2 animate-pulse"></div>
                    </div>
                </Transition>
                <TimedNotification
                    :is-ready="isReadyBundled" 
                    :duration="1500"
                    container-class="text-sm text-gray-500 dark:text-gray-400"
                > 
                    Game list from bundle pre-loaded <span class="text-green-400">✓</span>
                </TimedNotification>

            </div>
        </Transition>

        <!-- Search Bar -->
        <div class="mb-8">
            <div class="relative" ref="searchResultContainerRef">
               <div>
                 <input v-model="searchQuery" type="text" placeholder="Search Discord Verified games..."
                    class="w-full px-4 py-2 border border-gray-300 dark:border-gray-600 rounded-lg focus:ring-2 focus:ring-indigo-500 focus:border-indigo-500 dark:bg-gray-700 dark:text-white"
                    @focus="openSearchResults" />

                <!-- buttons to refetch game list -->
                <button
                    @click="fetchGameList()"
                    class="absolute right-0 top-1/2 transform -translate-y-1/2 px-3 mr-2 py-1 text-sm bg-gray-200 dark:bg-gray-600 hover:bg-gray-300 dark:hover:bg-gray-500 text-gray-700 dark:text-white rounded-md">
                    <span class="wrap whitespace-nowrap text-xs">
                        Refetch Game List
                    </span>
                </button>   
               </div>
                <div v-if="searchResultsIsOpen"
                    class="absolute z-50 mt-1 w-full bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg shadow-lg max-h-60 overflow-y-auto">
                    <div v-if="searchResults.length > 0">
                        <div v-for="game in searchResults" :key="game.item.id"
                            class="p-3 hover:bg-gray-100 dark:hover:bg-gray-700 border-b border-gray-200 dark:border-gray-700 last:border-b-0">
                            <div class="flex justify-between items-center">
                                <div>
                                    <div class="font-medium text-gray-800 dark:text-white">
                                        {{ game.item.name }}
                                    </div>
                                    <div class="text-sm text-gray-500 dark:text-gray-400">ID: {{ game.item.id }}</div>
                                    <div class="text-xs text-gray-500 dark:text-gray-400" v-if="gameHasWindowsExecutables(game.item)">
                                        Executables:
                                        <ul class="list-disc list-inside">
                                            <li v-for="exe in game.item.executables" :key="exe.name"
                                                class="text-gray-500 dark:text-gray-400">
                                                <span class="font-mono">
                                                {{ exe.name }}
                                                ({{ exe.os }})</span>
                                            </li>
                                        </ul>
                                    </div>
                                    <div class="text-xs text-amber-600 dark:text-amber-400 mt-1" v-else>
                                        No executables from Discord — needs Steam workaround
                                    </div>
                                </div>
                                <button @click="addGameToList(game.item)"
                                    class="ml-2 px-3 py-1 text-sm bg-indigo-600 hover:bg-indigo-700 text-white rounded-md">
                                    Add game to list
                                </button>
                            </div>
                        </div>
                    </div>
                    <!-- Some help -->
                    <div v-if="searchResults.length === 0"
                        class="p-3 hover:bg-gray-100 dark:hover:bg-gray-700 border-b border-gray-200 dark:border-gray-700 last:border-b-0 text-gray-500 dark:text-gray-400">
                        Search for games by name. <br>
                        Click "Add game to list" to add them to your selected games.
                    </div>
                </div>
            </div>
        </div>

        <!-- Two-Column Layout with right fixed column -->
        <div class="grid grid-cols-1 md:grid-cols-2 gap-6 relative">
            <!-- Left Column: Selected Games (scrollable) -->
            <!--  max-h-[70vh] overflow-y-auto : add these somewhere to just scroll the content  -->
            <div class="bg-white dark:bg-gray-800 p-4 rounded-lg shadow">
                <h2
                    class="text-xl font-bold text-gray-900 dark:text-white mb-4 sticky top-0 bg-white dark:bg-gray-800 py-2 z-10">
                    Games</h2>
                <div v-if="gameList.length === 0" class="text-gray-500 dark:text-gray-400 text-center py-8">
                    No games selected. Search and add games from the search bar.
                </div>
                <div v-else class="space-y-4">
                    <div v-for="game in gameList" :key="game.id" 
                        class="p-3 border border-gray-200 dark:border-gray-700 rounded-lg
                        hover:bg-gray-100 dark:hover:bg-gray-700/50 transition-colors 
                        duration-200 ease-in-out" 
                        :class="[
                            {
                                'ring-1 ring-violet-500/40 shadow-[0px_0px_8px_2px_#8e51ff50] bg-gray-100 dark:bg-gray-700/40': selectedGame?.uid === game.uid,
                            }
                        ]" @click="selectGame(game)"
                    >
                        <div class="flex justify-between items-center">
                            <div class="flex items-center gap-1">
                                <div class="font-medium text-gray-800 dark:text-white">{{ game.name }}</div>
                                <div class="relative inline-flex items-center">
                                    <div class="w-2 h-2 bg-white absolute rounded-full" style="left: 50%; top: 50%; transform: translate(-50%, -50%)"></div>
                                    <div class="relative inline-block">
                                     <IconVerified class="w-5 h-5 text-indigo-500 dark:text-indigo-400"></IconVerified>
                                    </div>
                                </div>
                            </div>
                            <button @click="removeGameFromList(game)" class="text-red-300 hover:text-red-400"
                                v-if="!game.is_running"> 
                                Remove
                            </button>
                        </div>
                        <div class="flex space-x-2 mt-2">
                            <!-- Previously play button was here -->
                            <div class="text-sm text-green-500 dark:text-green-400" v-if="game.is_running">
                                Running
                            </div>
                        </div>
                    </div>
                </div>
            </div>

            <!-- Right Column: Game Actions (fixed position) -->
            <div class="bg-white dark:bg-gray-800 p-4 rounded-lg shadow md:sticky md:top-4 self-start">
                <h2 class="text-xl font-bold text-gray-900 dark:text-white mb-4">Game Actions</h2>
                <div class="space-y-4">
                    <div class="text-gray-500 dark:text-gray-400 mb-2 text-sm" v-if="!selectedGame || selectedGame === null">
                        Select a game from the left to perform actions.
                    </div>
                    
                    <div v-if="selectedGame" class="text-gray-500 dark:text-gray-400 mb-4 text-sm">
                        <strong>Name:</strong> {{ selectedGame.name }}<br>
                        <strong>ID:</strong> {{ selectedGame.id }}<br>
                        <strong v-if="selectedGame.aliases && selectedGame.aliases.length > 0">Aliases:</strong>
                        <ul v-if="selectedGame.aliases && selectedGame.aliases.length > 0" class="list-disc list-inside" >
                            <li v-for="alias in selectedGame.aliases" :key="alias"
                                class="text-gray-500 dark:text-gray-400">
                                <span class="font-mono">{{ alias }}</span>
                            </li>
                        </ul>
                    </div>
                    <button @click="handleTestRPC(selectedGame)"
                        class="w-full py-2 rounded-lg bg-gray-600 hover:bg-gray-700 text-white">
                        {{ isConnecting ? 'Connecting...' : isConnectedToRPC ? 'Disconnect from Discord Gateway' : 'Test RPC' }}
                    </button>

                    <!-- divider -->
                    <div class="border-t border-gray-200 dark:border-gray-700 my-4"></div>

                    <GameExecutables v-if="selectedGame && selectedGameHasWinExes" :game="selectedGame" 
                        @play="playGame"
                        @stop="stopPlaying"
                        @install_and_play="installAndPlay"
                    />
                    <SteamQuestHelper v-else-if="selectedGame" :game="selectedGame"
                        @running-change="handleSteamRunningChange"
                    />
                </div>

                <!-- Divider -->
                <div class="border-t border-gray-200 dark:border-gray-700 my-5"></div>

                <div class="mt-6 p-4 border border-gray-200 dark:border-gray-700 rounded-lg">
                    <h3 class="font-medium text-gray-800 dark:text-white mb-2">Status</h3>
                    <div class="text-sm text-gray-500 dark:text-gray-400 mb-2">
                        Check Discord to see if it displays that you are playing a game.
                    </div>
                    <div v-if="currentlyPlaying" class="text-gray-500 dark:text-gray-400">
                        Currently playing: <span class="text-green-600"> {{gameList.find(g => g.id ===
                            currentlyPlaying)?.name }}</span>
                    </div>
                    <div v-else class="text-gray-500 dark:text-gray-400">
                        Not playing any game
                    </div>
                </div>

            </div>
        </div>
    </div>
</template>

<style scoped>
@reference "../theme/style.css";

.dialogStyle::backdrop {
    @apply bg-black/70 backdrop-blur-xs;
}
</style>
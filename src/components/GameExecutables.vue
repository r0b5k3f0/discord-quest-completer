<template>
    <div class="text-gray-500 dark:text-gray-400">
        <h3>
            The game has multiple platform executables. Please select one to launch:
        </h3>

        <div class="text-xs mt-2">
            <div v-for="(executable) in filteredExecutables" :key="executable.name"
                class="grid grid-cols-[auto_1fr_auto] gap-2 items-center mb-2 w-full">
                <div class="w-14 max-w-[80px]">
                    <div class="bg-gray-200 dark:bg-gray-700 rounded-full px-2 py-1 w-fit">
                        {{ executable.os }}
                    </div>
                </div>

                <!-- Sections / Breadcrumbs must fade when too long -->
                <div class="relative overflow-hidden ">
                    <div class="flex flex-nowrap overflow-x-auto scrollbar-none max-w-full pr-4 fade-right">
                        <div v-for="(section, i) in splitExecutableName(executable)" :key="i"
                            class="text-center border border-gray-300 dark:border-gray-700 rounded-md px-2 py-1 mr-1 whitespace-nowrap">
                            <span>{{ section }}</span>
                        </div>
                    </div>
                </div>

                <div class="justify-self-end">
                    <button class="text-white rounded-md px-3 py-1"
                    :class="[
                        {
                            'bg-blue-500 hover:bg-blue-600': !gameActions?.isExecutableRunning(executable),
                            'bg-red-500 hover:bg-red-600': gameActions?.isExecutableRunning(executable),
                        },
                    ]"
                        @click="handleLaunch(executable)"
                    >
                        {{ gameActions?.isExecutableRunning(executable) ? 'Stop' : 'Play' }}
                    </button>
                </div>
            </div>
        </div>
    </div>
</template>

<script setup lang="ts">
import { EXECUTABLE_OS, GameActionsKey } from '@/constants/constants';
import { GameActionsProvider, type Game, type GameExecutable } from '@/types/types';
import { path } from '@tauri-apps/api';
import { computed, inject } from 'vue';

const props = defineProps<{
    game: Game
}>();

const emit = defineEmits<{
    play: [{game: Game, executable: GameExecutable}]
    stop: [{game: Game, executable: GameExecutable}]
    install_and_play: [{game: Game, executable: GameExecutable}]
}>();

const gameActions = inject<GameActionsProvider>(GameActionsKey);

const filteredExecutables = computed(() => {
    return props.game.executables.filter(executable => {
        // currently no support for linux and darwin
        return executable.os !== EXECUTABLE_OS.LINUX && executable.os !== EXECUTABLE_OS.DARWIN
            && !hasIllegalChars(executable.name);
    });
});

const PATH_SEPARATOR = /\\|\//;

/** Breadcrumb sections for an executable path, with the extension stripped off
 *  the last one. */
function splitExecutableName(executable: GameExecutable) {
    const sections = executable.name.split(PATH_SEPARATOR);
    const last = sections[sections.length - 1];
    // Keep the last section as-is when it carries no extension.
    const name = last?.split('.').slice(0, -1).join('.') || last;
    return [
        ...sections.slice(0, -1),
        name,
    ];
}

/** The launch payload the parent needs: the directory the dummy exe goes in
 *  and its file name. */
function toLaunchPayload(executable: GameExecutable): GameExecutable {
    const sections = executable.name.split(PATH_SEPARATOR);
    return {
        // Spread first: the computed fields below must win over anything the
        // source object happens to carry.
        ...executable,
        path: sections.slice(0, -1).join(path.sep()),
        filename: sections[sections.length - 1],
    };
}

function hasIllegalChars(name: string) {
    const illegalChars = ['>', '<', ':', '"', '|', '?', '*'];
    return illegalChars.some(char => name.includes(char));
}

function handleLaunch(executable: GameExecutable) {
    const payload = { game: props.game, executable: toLaunchPayload(executable) };

    if (executable.is_running) {
        emit('stop', payload);
    } else if (!gameActions?.isGameExecutableInstalled(executable)) {
        emit('install_and_play', payload);
    } else {
        emit('play', payload);
    }
}

</script>

<style scoped>
.fade-right {
    -webkit-mask-image: linear-gradient(to right, black 85%, transparent 100%);
    mask-image: linear-gradient(to right, black 85%, transparent 100%);
}

.scrollbar-none {
    scrollbar-width: none;
    -ms-overflow-style: none;
}

.scrollbar-none::-webkit-scrollbar {
    display: none;
}
</style>
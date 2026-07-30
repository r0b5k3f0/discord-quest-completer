
export interface GameExecutable {
  is_launcher: boolean;
  name: string;
  os: string;
  /** Derived at launch time: the file name part of `name`. */
  filename?: string;
  /** Derived at launch time: the directory part of `name`. */
  path?: string;
  is_running?: boolean;
  is_installed?: boolean;
}
export interface ThirdPartySku {
  distributor: string;
  /** Discord leaves this null for a fair number of entries. */
  id: string | null;
}

export interface Game {
    uid?: string;
    id: string;
    name: string;
    executables: GameExecutable[];
    aliases?: string[];
    themes?: string[];
    third_party_skus?: ThirdPartySku[];
    is_running?: boolean;
    is_installed?: boolean;
}

export interface SteamOverrideEntry {
  app_id: string;
  name: string;
  steam_app_id: string;
  install_dir: string;
  exe_names: string[];
  note?: string;
}

export interface SteamLibrary {
  root: string;
  steamapps: string;
}

export interface SteamDummyStatus {
  installed: boolean;
  acf_exists: boolean;
  marker_exists: boolean;
  folder: string;
  exes: string[];
}

/** One launch entry from Steam's appinfo (see appinfo.rs). */
export interface SteamLaunchOption {
  /** Path exactly as Steam lists it, e.g. `game\bin\win64\cs2.exe`. */
  executable: string;
  /** File name part only. */
  filename: string;
  /** Directory part relative to the install dir; empty when at the top level. */
  sub_dir: string;
  launch_type: string;
}

/** Result of the `fetch_steam_appinfo` command. */
export interface SteamAppInfo {
  steam_app_id: number;
  name?: string | null;
  install_dir?: string | null;
  launch: SteamLaunchOption[];
  gated: boolean;
}

/** Result of the `search_steam_apps` command. */
export interface SteamAppSearchResult {
  appid: string;
  name: string;
}

export interface GameActionsProvider {
  canPlayGame: (game: Game | null) => boolean;
  isGameInstalled: (game: Game | null) => boolean;
  isExecutableRunning: (executable: GameExecutable) => boolean;
  isGameExecutableInstalled: (executable: GameExecutable) => boolean;
}
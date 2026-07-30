import type { Game, SteamOverrideEntry } from '@/types/types';
import overridesJson from '@/assets/steam-overrides.json';

interface OverridesFile {
  overrides: SteamOverrideEntry[];
}

const entries: SteamOverrideEntry[] = (overridesJson as OverridesFile).overrides ?? [];

/**
 * Find community-maintained Steam data for a game.
 *
 * Matching strategy:
 *  1. By Steam AppID present in the game's `third_party_skus` (robust).
 *  2. By Discord application id (may lose precision in JS for huge snowflakes,
 *     so it is only a fallback).
 *  3. By exact (case-insensitive) name match.
 */
export function findSteamOverride(game: Game | null | undefined): SteamOverrideEntry | undefined {
  if (!game) return undefined;

  const steamSkus = getSteamSkus(game);

  for (const entry of entries) {
    if (steamSkus.includes(entry.steam_app_id)) return entry;
  }

  const gameId = String(game.id ?? '');
  for (const entry of entries) {
    if (gameId && entry.app_id === gameId) return entry;
  }

  const name = (game.name ?? '').trim().toLowerCase();
  if (name) {
    for (const entry of entries) {
      if (entry.name.trim().toLowerCase() === name) return entry;
    }
  }

  return undefined;
}

/** All Steam AppIDs Discord associates with this game. Entries with a null id
 *  are dropped rather than stringified into a bogus "null" AppID. */
export function getSteamSkus(game: Game | null | undefined): string[] {
  if (!game) return [];
  return (game.third_party_skus ?? [])
    .filter((sku) => (sku.distributor ?? '').toLowerCase() === 'steam')
    .map((sku) => (sku.id == null ? '' : String(sku.id)))
    .filter((id) => id.length > 0);
}

/** True when the game has at least one usable Windows executable from Discord. */
export function gameHasWindowsExecutables(game: Game | null | undefined): boolean {
  if (!game) return false;
  const illegalChars = ['>', '<', ':', '"', '|', '?', '*'];
  return (game.executables ?? []).some((exe) => {
    if (!exe?.name) return false;
    if (exe.os === 'linux' || exe.os === 'darwin') return false;
    if (illegalChars.some((c) => exe.name.includes(c))) return false;
    return true;
  });
}

/** Sanitize a string into something usable as a folder/exe base name. */
export function sanitizeSegment(value: string): string {
  return value
    .replace(/[<>:"|?*\\/]/g, '')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 64);
}

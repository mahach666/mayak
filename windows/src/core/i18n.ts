// Localization (batch spec §C): t(key, vars?) in the language chosen by setLanguage(). The pure
// part (resolveLanguage, format) is in core/i18n-core.ts so Node can check it; this file wires
// the dictionaries. A language change reloads the page, so nothing here re-renders.

import { format, resolveLanguage, type Lang, type Vars } from "./i18n-core";
import { en } from "./i18n/en";
import { ru } from "./i18n/ru";
import { de } from "./i18n/de";
import { fr } from "./i18n/fr";
import { es } from "./i18n/es";
import { zh } from "./i18n/zh";
import { ja } from "./i18n/ja";

export { LANGS, resolveLanguage, type Lang } from "./i18n-core";

export type Key = keyof typeof en;

const DICTIONARIES: Record<Lang, Record<Key, string>> = { en, ru, de, fr, es, zh, ja };

let current: Lang = "en";

/** "system" (or anything that is no language) follows navigator.language, else English. */
export function setLanguage(setting: string): void {
  current = resolveLanguage(setting, navigator.language);
  document.documentElement.lang = current;
}

export function currentLanguage(): Lang {
  return current;
}

export function t(key: Key, vars?: Vars): string {
  return format(DICTIONARIES[current][key], vars);
}

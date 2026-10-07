// The pure part of the localization (batch spec §C): no imports, so Node can run it as it is
// (C:\tmp\mayak-spike\i18n-check.mjs). The dictionaries and `t()` live in core/i18n.ts.

export type Lang = "en" | "ru" | "de" | "fr" | "es" | "zh" | "ja";

export const LANGS: readonly Lang[] = ["en", "ru", "de", "fr", "es", "zh", "ja"];

export type Vars = Record<string, string | number>;

/** A translator: `t()` itself, or the English default of a pure module that Node checks. */
export type Translate<K extends string = string> = (key: K, vars?: Vars) => string;

function isLang(value: string): value is Lang {
  return (LANGS as readonly string[]).includes(value);
}

/**
 * A language code from the setting: the code itself when it is one of the languages, else
 * ("system", or anything unknown) the language of `navigatorLanguage` ("ru-RU" → "ru") when it is
 * one, else English.
 */
export function resolveLanguage(setting: string, navigatorLanguage: string): Lang {
  if (isLang(setting)) return setting;
  const base = navigatorLanguage.split("-")[0].toLowerCase();
  return isLang(base) ? base : "en";
}

/** Replaces each `{name}` with its value; a name without a value stays as it is. */
export function format(template: string, vars: Vars = {}): string {
  return template.replace(/\{(\w+)\}/g, (whole, name: string) =>
    Object.hasOwn(vars, name) ? String(vars[name]) : whole,
  );
}

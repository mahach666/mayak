// Every dictionary has exactly en's keys, and each text keeps en's {placeholders}.
import assert from "node:assert/strict";
const base = new URL("../../src/core/i18n/", import.meta.url).href;
const { en } = await import(base + "en.ts");
const ph = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(",");
for (const lang of ["ru", "de", "fr", "es", "zh", "ja"]) {
  const dict = (await import(base + lang + ".ts"))[lang];
  assert.deepEqual(Object.keys(dict).sort(), Object.keys(en).sort(), `${lang}: keys differ from en`);
  for (const [k, v] of Object.entries(en)) {
    assert.equal(ph(dict[k]), ph(v), `${lang} ${k}: placeholders differ`);
    assert.ok(dict[k].trim().length > 0, `${lang} ${k}: empty`);
  }
}
for (const k of ["tray.open", "tray.settings", "tray.focus", "tray.pause", "tray.quit", "tray.waiting", "tray.minutes", "tray.justNow"]) {
  assert.ok(k in en, `en has no ${k}`);
  assert.ok(en[k].length <= 64, `${k} is longer than the tray's 64 characters`);
}
console.log(`i18n-keys-check: all passed (${Object.keys(en).length} keys)`);

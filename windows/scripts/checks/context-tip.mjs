// views/context-tip.ts: the token count, the percent of a context window and the tooltip of a
// session's context label (with the model of a Claude session).
import assert from "node:assert/strict";
import * as C from "../../src/views/context-tip.ts";
import { en } from "../../src/core/i18n/en.ts";
import { ru } from "../../src/core/i18n/ru.ts";
import { format } from "../../src/core/i18n-core.ts";

const tr = (k, v) => format(en[k], v);
const trRu = (k, v) => format(ru[k], v);

// ── Token counts, always rounded down ──
assert.deepEqual([950, 1000, 612_999, 999_999, 1_000_000, 1_299_999].map(C.formatTokens), ["950", "1k", "612k", "999k", "1.0M", "1.2M"]);

// ── Percent, on integers, at most 100, null without a window ──
assert.equal(C.contextPercent({ used: 612_000, window: 1_000_000 }), 61);
assert.equal(C.contextPercent({ used: 29, window: 100 }), 29, "not 28 from 0.29 * 100");
assert.equal(C.contextPercent({ used: 1_200_000, window: 1_000_000 }), 100);
assert.equal(C.contextPercent({ used: 612_000 }), null);
assert.equal(C.contextPercent({ used: 612_000, window: 0 }), null);

// ── Tooltip ──
assert.equal(C.contextTip({ used: 612_000, window: 1_000_000 }, "claude-opus-5-5", tr), "Context: 612k of 1.0M (61%)\nModel: claude-opus-5-5");
assert.equal(C.contextTip({ used: 150_000, window: 200_000 }, undefined, tr), "Context: 150k of 200k (75%)", "Codex: no model line");
assert.equal(C.contextTip({ used: 90_000 }, "claude-next-9", tr), "Context: 90k tokens\nModel: claude-next-9", "an unknown model: the count alone");
assert.equal(C.contextTip({ used: 90_000 }, "", tr), "Context: 90k tokens");
assert.equal(C.contextTip({ used: 612_000, window: 1_000_000 }, "claude-opus-5-5", trRu), "Контекст: 612k из 1.0M (61%)\nМодель: claude-opus-5-5");

console.log("context-tip-check: all passed");

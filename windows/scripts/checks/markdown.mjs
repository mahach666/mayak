// core/markdown.ts: what the Agents chat reads of an agent's answer.
// Run: node --experimental-strip-types markdown.mjs
import assert from "node:assert/strict";
import { inlineText, parseInline, parseMarkdown, safeWebUrl } from "../../src/core/markdown.ts";

const kinds = (src) => parseMarkdown(src).map((b) => b.t);
const text = (nodes) => inlineText(nodes);

// Blocks.
assert.deepEqual(kinds("# Title\n\nSome text\nmore text\n\n---\n\n> quoted"), ["heading", "para", "rule", "quote"]);
assert.equal(parseMarkdown("## Two **bold**")[0].level, 2);
assert.equal(text(parseMarkdown("### Closed ###")[0].c), "Closed");
assert.deepEqual(kinds("#hashtag is a paragraph"), ["para"]);
assert.deepEqual(kinds("#"), [], "an empty heading draws nothing");
const para = parseMarkdown("one\ntwo\nthree")[0];
assert.equal(text(para.c), "one\ntwo\nthree", "a paragraph keeps its line breaks");
assert.deepEqual(kinds("one\n- two"), ["para", "item"], "a list ends a paragraph");
assert.deepEqual(kinds("one\n```\ncode\n```\ntwo"), ["para", "code", "para"]);

// Fenced code: its language, its text untouched, an open fence runs to the end (a half-written answer).
const fenced = parseMarkdown("```ts\nconst a = 1;\n  // **not bold**\n```");
assert.deepEqual(fenced, [{ t: "code", lang: "ts", s: "const a = 1;\n  // **not bold**" }]);
assert.deepEqual(parseMarkdown("```\nopen\nstill open"), [{ t: "code", lang: "", s: "open\nstill open" }]);
assert.deepEqual(parseMarkdown("~~~\nx\n~~~"), [{ t: "code", lang: "", s: "x" }]);
assert.equal(parseMarkdown("````\n```\ninner\n```\n````")[0].s, "```\ninner\n```", "a longer fence holds a shorter one");

// Lists: bullets, numbers, nesting by indent.
const list = parseMarkdown("- a\n  - b\n1. c\n10) d\n* e");
assert.deepEqual(list.map((b) => [b.marker, b.indent]), [["•", 0], ["•", 1], ["1.", 0], ["10.", 0], ["•", 0]]);
assert.equal(parseMarkdown("2 * 3 * 4")[0].t, "para", "a star inside a line is not a bullet");
assert.equal(text(parseMarkdown("2 * 3 * 4")[0].c), "2 * 3 * 4");

// Quotes merge; a table has a head and padded rows; a lone pipe line is text.
assert.equal(parseMarkdown("> a\n> b")[0].t, "quote");
assert.equal(text(parseMarkdown("> a\n> b")[0].c), "a\nb");
const table = parseMarkdown("| Name | Size |\n|---|:--:|\n| a | 1 |\n| b |\n\nafter")[0];
assert.equal(table.t, "table");
assert.deepEqual(table.head.map(text), ["Name", "Size"]);
assert.deepEqual(table.rows.map((r) => r.map(text)), [["a", "1"], ["b", ""]]);
assert.deepEqual(kinds("a | b\nc"), ["para"]);
assert.deepEqual(kinds("| x |\n|---|\n| 1 |\n\ntext"), ["table", "para"]);

// Inline: code, bold, italic, both, links, escapes.
const types = (s) => parseInline(s).map((n) => n.t);
assert.deepEqual(types("a `b` c"), ["text", "code", "text"]);
assert.equal(parseInline("`` a ` b ``")[0].s, "a ` b");
assert.deepEqual(types("**b** and *i* and _j_"), ["strong", "text", "em", "text", "em"]);
assert.deepEqual(types("***both***"), ["strong"]);
assert.equal(parseInline("***both***")[0].c[0].t, "em");
assert.equal(text(parseInline("a **b *c* d** e")), "a b c d e");
assert.deepEqual(types("snake_case_name stays"), ["text"], "an underscore inside a word is text");
assert.deepEqual(types("2 * 3 and a*b*c"), ["text", "em", "text"], "a star needs no space-free edge; one inside a word may open");
assert.equal(text(parseInline("2 * 3")), "2 * 3");
assert.deepEqual(types("unclosed **bold"), ["text"], "an unfinished emphasis stays as typed");
assert.equal(text(parseInline("unclosed **bold")), "unclosed **bold");
assert.equal(text(parseInline("\\*not\\* \\`code\\`")), "*not* `code`");
assert.deepEqual(types("`**x**` **`y`**"), ["code", "text", "strong"], "code is not read as emphasis");

// Links: web addresses only.
const link = parseInline("see [the docs](https://example.com/a?b=1) now")[1];
assert.equal(link.t, "link");
assert.equal(link.href, "https://example.com/a?b=1");
assert.equal(text(link.c), "the docs");
assert.deepEqual(types("[x](javascript:alert(1))"), ["text"], "a script link is only its text");
assert.equal(text(parseInline("[x](javascript:alert(1))")), "x");
assert.equal(text(parseInline("[x](file:///c:/secret)")), "x");
assert.equal(text(parseInline("[x](data:text/html,hi)")), "x");
const bare = parseInline("go to https://example.com/x, then (https://a.b/c).");
assert.deepEqual(bare.filter((n) => n.t === "link").map((n) => n.href), ["https://example.com/x", "https://a.b/c"]);
assert.equal(text(bare), "go to https://example.com/x, then (https://a.b/c).");
assert.equal(safeWebUrl("https://example.com"), "https://example.com");
assert.equal(safeWebUrl("HTTP://example.com/a"), "HTTP://example.com/a");
for (const bad of ["javascript:alert(1)", "ftp://x", "https://a b", "//x.y", "https://", "", "https://x\"onload=1"]) {
  assert.equal(safeWebUrl(bad), null, bad);
}

// What an agent writes is never HTML here: tags are plain text.
assert.equal(text(parseInline("<img src=x onerror=alert(1)> <b>x</b>")), "<img src=x onerror=alert(1)> <b>x</b>");

// A long answer is read in a moment (no quadratic scan).
const long = ("word http-ish h ".repeat(2000) + "**bold** `c` [l](https://x.y) ").repeat(20);
const t0 = performance.now();
parseMarkdown(long);
assert.ok(performance.now() - t0 < 1500, `slow: ${performance.now() - t0} ms`);

console.log("markdown-check: all passed");

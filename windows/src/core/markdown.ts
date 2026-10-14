// A small Markdown reader for what an agent writes in the Agents chat: headings, paragraphs, fenced
// code, lists, quotes, rules, tables, and inline code, bold, italic and links. No DOM: it gives a
// tree, and views/markdown-view.ts draws it with textContent only, so nothing an agent writes is
// ever read as HTML. The block set is the one the original chat reads (ChatMarkdown.swift) plus
// tables. A half-written answer (an open code fence, an unfinished `**`) parses as far as it goes.

export type Inline =
  | { t: "text"; s: string }
  | { t: "code"; s: string }
  | { t: "strong"; c: Inline[] }
  | { t: "em"; c: Inline[] }
  | { t: "link"; href: string; c: Inline[] };

export type Block =
  | { t: "heading"; level: number; c: Inline[] }
  | { t: "para"; c: Inline[] }
  | { t: "code"; lang: string; s: string }
  /** One list line: `marker` is "•" or "3.", `indent` the nesting level (0-based). */
  | { t: "item"; marker: string; indent: number; c: Inline[] }
  | { t: "quote"; c: Inline[] }
  | { t: "rule" }
  | { t: "table"; head: Inline[][]; rows: Inline[][][] };

/** Only plain web links are ever opened; anything else (javascript:, file:, data:) is shown as text. */
export function safeWebUrl(raw: string): string | null {
  const url = raw.trim();
  if (!/^https?:\/\//i.test(url) || /[\s<>"]/.test(url)) return null;
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:" ? url : null;
  } catch {
    return null;
  }
}

const MAX_NEST = 4;
const FENCE = /^\s{0,3}(`{3,}|~{3,})\s*([^\s`]*)/;
const HEADING = /^\s{0,3}(#{1,6})(?:\s+(.*?))?\s*$/;
const RULE = /^\s{0,3}(?:-{3,}|\*{3,}|_{3,})\s*$/;
const BULLET = /^(\s*)[-*+]\s+(.*)$/;
const ORDERED = /^(\s*)(\d{1,9})[.)]\s+(.*)$/;
const TABLE_SEP = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

function closesFence(line: string, open: string): boolean {
  const m = /^\s{0,3}(`{3,}|~{3,})\s*$/.exec(line);
  return m !== null && m[1][0] === open[0] && m[1].length >= open.length;
}

function isBlockStart(line: string, next: string | undefined): boolean {
  return (
    FENCE.test(line) || /^\s{0,3}#{1,6}(\s|$)/.test(line) || RULE.test(line) || /^\s{0,3}>/.test(line) ||
    BULLET.test(line) || ORDERED.test(line) || (line.includes("|") && next !== undefined && TABLE_SEP.test(next) && next.includes("-"))
  );
}

function splitRow(line: string): string[] {
  let body = line.trim();
  if (body.startsWith("|")) body = body.slice(1);
  if (body.endsWith("|") && !body.endsWith("\\|")) body = body.slice(0, -1);
  const cells: string[] = [];
  let cell = "";
  for (let i = 0; i < body.length; i++) {
    if (body[i] === "\\" && body[i + 1] === "|") {
      cell += "|";
      i++;
    } else if (body[i] === "|") {
      cells.push(cell.trim());
      cell = "";
    } else cell += body[i];
  }
  cells.push(cell.trim());
  return cells;
}

export function parseMarkdown(source: string): Block[] {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const fence = FENCE.exec(line);
    if (fence) {
      const code: string[] = [];
      i++;
      // A fence closes with the same character, at least as long; an unclosed one runs to the end.
      while (i < lines.length && !closesFence(lines[i], fence[1])) {
        code.push(lines[i]);
        i++;
      }
      i++;
      blocks.push({ t: "code", lang: fence[2], s: code.join("\n") });
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      const text = (heading[2] ?? "").replace(/\s+#+$/, "").trim();
      if (text !== "") blocks.push({ t: "heading", level: heading[1].length, c: parseInline(text) });
      i++;
      continue;
    }
    if (RULE.test(line)) {
      blocks.push({ t: "rule" });
      i++;
      continue;
    }
    if (/^\s{0,3}>/.test(line)) {
      const quoted: string[] = [];
      while (i < lines.length && /^\s{0,3}>/.test(lines[i])) {
        quoted.push(lines[i].replace(/^\s{0,3}>\s?/, ""));
        i++;
      }
      blocks.push({ t: "quote", c: parseInline(quoted.join("\n")) });
      continue;
    }
    const bullet = BULLET.exec(line);
    const ordered = bullet ? null : ORDERED.exec(line);
    if (bullet || ordered) {
      const m = (bullet ?? ordered) as RegExpExecArray;
      blocks.push({
        t: "item",
        marker: bullet ? "•" : `${m[2]}.`,
        indent: Math.min(MAX_NEST, Math.floor(m[1].replace(/\t/g, "  ").length / 2)),
        c: parseInline(bullet ? m[2] : m[3]),
      });
      i++;
      continue;
    }
    if (line.includes("|") && i + 1 < lines.length && TABLE_SEP.test(lines[i + 1]) && lines[i + 1].includes("-")) {
      const head = splitRow(line).map(parseInline);
      const rows: Inline[][][] = [];
      i += 2;
      while (i < lines.length && lines[i].trim() !== "" && lines[i].includes("|")) {
        const cells = splitRow(lines[i]).map(parseInline);
        while (cells.length < head.length) cells.push([]);
        rows.push(cells.slice(0, head.length));
        i++;
      }
      blocks.push({ t: "table", head, rows });
      continue;
    }
    if (line.trim() === "") {
      i++;
      continue;
    }
    // A paragraph: the lines up to the next blank line or the next block.
    const para: string[] = [line];
    i++;
    while (i < lines.length && lines[i].trim() !== "" && !isBlockStart(lines[i], lines[i + 1])) {
      para.push(lines[i]);
      i++;
    }
    blocks.push({ t: "para", c: parseInline(para.join("\n")) });
  }
  return blocks;
}

const URL_AT = /https?:\/\/[^\s<>"'`]+/iy;
const PUNCT = /[\\`*_{}[\]()#+\-.!|~<>]/;
const isWordChar = (ch: string | undefined): boolean => ch !== undefined && /[\p{L}\p{N}]/u.test(ch);

/**
 * The first run of exactly `n` of `mark` at or after `from` (a longer or shorter run is skipped as
 * a whole; escapes and code spans are stepped over), or -1.
 */
function findClose(s: string, mark: string, n: number, from: number): number {
  let j = from;
  while (j < s.length) {
    if (s[j] === "\\") {
      j += 2;
    } else if (s[j] === "`") {
      let run = 1;
      while (s[j + run] === "`") run++;
      const end = s.indexOf("`".repeat(run), j + run);
      j = end < 0 ? j + run : end + run;
    } else if (s[j] === mark) {
      let run = 1;
      while (s[j + run] === mark) run++;
      if (run === n) return j;
      j += run;
    } else j++;
  }
  return -1;
}

/** The `)` that closes a link's address starting at `from` (an address may hold balanced parentheses), or -1. */
function closingParen(s: string, from: number): number {
  let depth = 0;
  for (let j = from; j < s.length; j++) {
    if (s[j] === "\n") return -1;
    if (s[j] === "(") depth++;
    else if (s[j] === ")" && depth-- === 0) return j;
  }
  return -1;
}

export function parseInline(s: string): Inline[] {
  const out: Inline[] = [];
  let text = "";
  const flush = () => {
    if (text !== "") out.push({ t: "text", s: text });
    text = "";
  };
  let i = 0;
  while (i < s.length) {
    const ch = s[i];
    if (ch === "\\" && i + 1 < s.length && PUNCT.test(s[i + 1])) {
      text += s[i + 1];
      i += 2;
      continue;
    }
    if (ch === "`") {
      let n = 1;
      while (s[i + n] === "`") n++;
      const mark = "`".repeat(n);
      const end = s.indexOf(mark, i + n);
      if (end > i + n && s[end + n] !== "`") {
        flush();
        let code = s.slice(i + n, end);
        if (code.length > 2 && code.startsWith(" ") && code.endsWith(" ")) code = code.slice(1, -1);
        out.push({ t: "code", s: code });
        i = end + n;
        continue;
      }
      text += mark;
      i += n;
      continue;
    }
    if (ch === "[") {
      const close = s.indexOf("](", i + 1);
      const end = close < 0 ? -1 : closingParen(s, close + 2);
      if (close > i + 1 && end > close + 2 && !s.slice(i + 1, close).includes("\n")) {
        const href = safeWebUrl(s.slice(close + 2, end).replace(/\s+"[^"]*"$/, ""));
        const inner = parseInline(s.slice(i + 1, close));
        flush();
        if (href) out.push({ t: "link", href, c: inner });
        else out.push(...inner);
        i = end + 1;
        continue;
      }
    }
    if (ch === "*" || ch === "_") {
      let n = 1;
      while (s[i + n] === ch) n++;
      // `_` opens only at a word edge (snake_case_name is text); `*` may sit inside a word.
      if (n <= 3 && (ch === "*" || !isWordChar(s[i - 1]))) {
        const end = findClose(s, ch, n, i + n);
        const inner = end < 0 ? "" : s.slice(i + n, end);
        if (end > i + n && !/^\s|\s$/.test(inner) && (ch === "*" || !isWordChar(s[end + n]))) {
          flush();
          const kids = parseInline(inner);
          out.push(n === 1 ? { t: "em", c: kids } : n === 2 ? { t: "strong", c: kids } : { t: "strong", c: [{ t: "em", c: kids }] });
          i = end + n;
          continue;
        }
      }
      text += ch.repeat(n);
      i += n;
      continue;
    }
    if ((ch === "h" || ch === "H") && !isWordChar(s[i - 1])) {
      URL_AT.lastIndex = i;
      const m = URL_AT.exec(s);
      if (m) {
        const url = m[0].replace(/[.,;:!?)\]]+$/, "");
        const href = safeWebUrl(url);
        if (href) {
          flush();
          out.push({ t: "link", href, c: [{ t: "text", s: url }] });
          i += url.length;
          continue;
        }
      }
    }
    text += ch;
    i++;
  }
  flush();
  return out;
}

/** The plain text of inline nodes (a table cell's, a heading's), for tests and tooltips. */
export function inlineText(nodes: readonly Inline[]): string {
  return nodes.map((n) => (n.t === "text" || n.t === "code" ? n.s : inlineText(n.c))).join("");
}

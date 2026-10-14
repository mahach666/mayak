// Draws what core/markdown.ts parsed. Text goes in through textContent only, so nothing an agent
// writes is ever taken for HTML, and a link has no href: a click hands its address to the app, which
// opens web addresses and nothing else (Bridge.openUrl), instead of the page going there itself.

import { h, svg } from "./dom";
import { ICONS } from "./icons";
import { parseMarkdown, type Block, type Inline } from "../core/markdown";
import { t } from "../core/i18n";

export interface MarkdownHooks {
  openUrl(url: string): void;
  copy(text: string): Promise<void>;
}

/** How long a code block's Copy shows its answer (a tick or a cross) before it is a button again. */
const COPY_ANSWER_MS = 1500;

function inline(nodes: readonly Inline[], hooks: MarkdownHooks): Node[] {
  return nodes.map((n): Node => {
    switch (n.t) {
      case "text": return document.createTextNode(n.s);
      case "code": return h("code", { class: "md-ic", text: n.s });
      case "strong": return h("strong", {}, ...inline(n.c, hooks));
      case "em": return h("em", {}, ...inline(n.c, hooks));
      case "link": {
        const a = h("a", { class: "md-a", title: n.href, role: "link", tabindex: 0 }, ...inline(n.c, hooks));
        const open = () => hooks.openUrl(n.href);
        a.addEventListener("click", open);
        a.addEventListener("keydown", (e) => {
          if (e.key === "Enter") open();
        });
        return a;
      }
    }
  });
}

function copyButton(code: string, hooks: MarkdownHooks): HTMLElement {
  const button = h("button", { class: "md-copy", title: t("agentchat.copy") });
  const draw = (state: "idle" | "ok" | "failed") => {
    button.replaceChildren(
      state === "ok" ? svg(ICONS.check, 12, { stroke: 2.4 })
      : state === "failed" ? svg(ICONS.xmark, 10)
      : svg(ICONS.copy, 12),
    );
    button.classList.toggle("ok", state === "ok");
    button.classList.toggle("failed", state === "failed");
    button.title = state === "ok" ? t("agentchat.copied") : state === "failed" ? t("agentchat.copyFailed") : t("agentchat.copy");
  };
  draw("idle");
  button.addEventListener("click", () => {
    const answer = (state: "ok" | "failed") => {
      draw(state);
      window.setTimeout(() => draw("idle"), COPY_ANSWER_MS);
    };
    hooks.copy(code).then(() => answer("ok"), () => answer("failed"));
  });
  return button;
}

function block(b: Block, hooks: MarkdownHooks): HTMLElement {
  switch (b.t) {
    case "heading": return h("div", { class: `md-h md-h${Math.min(b.level, 3)}` }, ...inline(b.c, hooks));
    case "para": return h("div", { class: "md-p" }, ...inline(b.c, hooks));
    case "code":
      return h("div", { class: "md-code" },
        h("div", { class: "md-code-head" }, h("span", { class: "md-lang", text: b.lang }), copyButton(b.s, hooks)),
        h("pre", {}, h("code", { text: b.s })));
    case "item":
      return h("div", { class: "md-li", style: `margin-left:${b.indent * 14}px` },
        h("span", { class: `md-mark${b.marker === "•" ? "" : " num"}`, text: b.marker }),
        h("div", { class: "md-li-body" }, ...inline(b.c, hooks)));
    case "quote": return h("div", { class: "md-quote" }, ...inline(b.c, hooks));
    case "rule": return h("hr", { class: "md-hr" });
    case "table":
      return h("div", { class: "md-table-wrap" },
        h("table", { class: "md-table" },
          h("thead", {}, h("tr", {}, ...b.head.map((c) => h("th", {}, ...inline(c, hooks))))),
          h("tbody", {}, ...b.rows.map((r) => h("tr", {}, ...r.map((c) => h("td", {}, ...inline(c, hooks))))))));
  }
}

/** One agent answer as Markdown. */
export function renderMarkdown(source: string, hooks: MarkdownHooks): HTMLElement {
  return h("div", { class: "md" }, ...parseMarkdown(source).map((b) => block(b, hooks)));
}

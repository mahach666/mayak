// The Stats tab (night spec §C): tokens of the last 7 or 30 days by day, project, agent and model.
// Rust counts them from the agents' files each time the tab opens (stats_query, on its own
// thread); nothing runs while the tab is closed, and nothing goes through State.notify().

import { h, clear } from "./dom";
import { Bridge, type StatsReport } from "../core/bridge";
import { formatTokens } from "./sessions";
import { t } from "../core/i18n";
import type { ViewHost } from "./views";

const AGENT_NAME = { claude: "Claude", codex: "Codex" } as const;
const TOP_PROJECTS = 5;

type Period = 7 | 30;

function dayBars(report: StatsReport): HTMLElement {
  const max = Math.max(0, ...report.days.map((d) => d.total));
  const last = report.days.length - 1;
  const bars = h("div", { class: "stats-bars" });
  report.days.forEach((d, i) => {
    const fill = h("i", { class: i === last ? "stats-fill today" : "stats-fill" });
    // A day with any use keeps a visible sliver next to a much larger one.
    fill.style.height = d.total > 0 && max > 0 ? `max(2px, ${(d.total / max) * 100}%)` : "0";
    bars.append(h("div", { class: "stats-bar", title: `${d.day} · ${formatTokens(d.total)}` }, fill));
  });
  return bars;
}

function projectList(report: StatsReport): HTMLElement {
  const el = h(
    "div",
    { class: "stats-col" },
    h("div", { class: "stats-line stats-name" }, h("span", { class: "stats-label", text: t("stats.projects") })),
  );
  for (const p of report.projects.slice(0, TOP_PROJECTS)) {
    el.append(h(
      "div",
      { class: "stats-line", title: p.project },
      h("span", { class: "stats-label", text: p.name }),
      h("span", { class: "stats-num", text: formatTokens(p.total) }),
    ));
  }
  return el;
}

function agentTable(report: StatsReport): HTMLElement {
  const el = h(
    "div",
    { class: "stats-col wide" },
    h(
      "div",
      { class: "stats-line stats-name" },
      h("span", { class: "stats-label", text: t("stats.agentModel") }),
      h("span", { class: "stats-num", title: t("stats.input"), text: t("stats.input") }),
      h("span", { class: "stats-num", title: t("stats.cache"), text: t("stats.cache") }),
      h("span", { class: "stats-num", title: t("stats.output"), text: t("stats.output") }),
    ),
  );
  for (const a of report.agents) {
    const cache = a.cacheRead + a.cacheWrite;
    el.append(h(
      "div",
      { class: "stats-line", title: `${AGENT_NAME[a.agent] ?? a.agent} · ${a.model}` },
      h(
        "span",
        { class: "stats-label" },
        h("span", { class: "stats-agent", text: AGENT_NAME[a.agent] ?? a.agent }),
        ` ${a.model}`,
      ),
      h("span", { class: "stats-num", text: formatTokens(a.input) }),
      h("span", { class: "stats-num", title: t("stats.cacheDetail", { read: formatTokens(a.cacheRead), write: formatTokens(a.cacheWrite) }), text: formatTokens(cache) }),
      h("span", { class: "stats-num", text: formatTokens(a.output) }),
    ));
  }
  return el;
}

export function buildStats(): ViewHost {
  const total = h("span", { class: "stats-total" });
  const caption = h("span", { class: "stats-caption" });
  const periodButtons = ([7, 30] as const).map((days) =>
    h("button", { onclick: () => open(days) }, t("stats.days", { n: days })),
  );
  const head = h("div", { class: "stats-head" }, total, caption, h("div", { class: "seg" }, ...periodButtons));
  const body = h("div", { class: "stats-body" });
  const el = h("div", { class: "view" }, h("div", { class: "card stats-card" }, h("div", { class: "stats" }, head, body)));

  let period: Period = 7;
  // Only the answer to the latest request is shown: a slow 30-day count must not land over a
  // 7-day one asked after it.
  let request = 0;

  function message(text: string) {
    clear(body);
    body.append(h("div", { class: "stats-empty", text }));
  }

  function render(report: StatsReport) {
    total.textContent = formatTokens(report.total.total);
    caption.textContent = t("stats.caption", { n: period });
    clear(body);
    if (report.total.total === 0) {
      body.append(dayBars(report), h("div", { class: "stats-empty", text: t("stats.none", { n: period }) }));
      return;
    }
    body.append(dayBars(report), h("div", { class: "stats-cols" }, projectList(report), agentTable(report)));
  }

  function open(days: Period) {
    period = days;
    periodButtons.forEach((b, i) => b.classList.toggle("on", [7, 30][i] === days));
    total.textContent = "";
    caption.textContent = "";
    message(t("stats.counting"));
    const mine = ++request;
    void Bridge.statsQuery(days).then((report) => {
      if (mine !== request) return;
      if (report) render(report);
      else message(t("stats.failed"));
    });
  }

  return {
    el,
    sync() {},
    setActive(on: boolean) {
      if (on) open(7);
      // An answer that lands after the tab was left is not shown.
      else request++;
    },
  };
}

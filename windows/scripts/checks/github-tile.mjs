// The GitHub tile (views/home-model.ts githubRows / githubHasPullRequests / tileAlert) and the
// account line of GitHub's Connect (settings/oauth-view.ts githubModeKey): review requests come
// first, then my pull requests whose CI failed; a failed CI tints the tile; each mode Rust sends
// (src-tauri oauth.rs github_mode) has a text of the dictionaries.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as HM from "../../src/views/home-model.ts";
import { githubModeKey } from "../../src/settings/oauth-view.ts";
import { en } from "../../src/core/i18n/en.ts";

// Shaped as src-tauri github.rs card_data.
const data = {
  totalRepos: 4, totalStars: 10, login: "octocat", note: null,
  mine: [
    { title: "Green", reference: "o/a#1", number: 1, url: "https://github.com/o/a/pull/1", draft: false, ci: "success" },
    { title: "Red draft", reference: "o/a#2", number: 2, url: "https://github.com/o/a/pull/2", draft: true, ci: "failed" },
    { title: "Red", reference: "o/a#3", number: 3, url: null, draft: false, ci: "failed" },
    { title: "No CI", reference: "o/a#4", number: 4, url: null, draft: false, ci: null },
  ],
  mineTotal: 4,
  reviews: [{ title: "Please review", reference: "o/b#9", number: 9, url: "https://github.com/o/b/pull/9", draft: false, ci: null }],
  reviewsTotal: 1,
  ci: { failed: 2, running: 0, success: 1 },
};

assert.equal(HM.githubHasPullRequests(data), true);
assert.equal(HM.githubHasPullRequests({ totalRepos: 4, totalStars: 0, note: "permissions" }), false);
assert.equal(HM.githubHasPullRequests(undefined), false);

assert.deepEqual(HM.githubRows(data, 2), [
  { kind: "review", title: "Please review", number: "#9", url: "https://github.com/o/b/pull/9", draft: false },
  { kind: "failed", title: "Red draft", number: "#2", url: "https://github.com/o/a/pull/2", draft: true },
]);
assert.deepEqual(HM.githubRows(data, 5).map((r) => r.number), ["#9", "#2", "#3"], "green and unknown CI are not rows");
assert.deepEqual(HM.githubRows(data, 1).map((r) => r.kind), ["review"]);
assert.deepEqual(HM.githubRows({ mine: [], reviews: [] }, 2), []);
assert.deepEqual(HM.githubRows({ mine: "x", reviews: null }, 2), [], "a broken payload is no rows, not a crash");
assert.equal(HM.githubRows({ reviews: [{ title: 1, number: "x", url: 5 }] }, 2)[0].url, null);

// A failed CI of mine tints the tile, as a failed GitLab pipeline does.
assert.equal(HM.tileAlert("integration_github", { state: "idle" }, data), true);
assert.equal(HM.tileAlert("integration_github", { state: "idle" }, { ...data, ci: { failed: 0, running: 1, success: 3 } }), false);
assert.equal(HM.tileAlert("integration_github", { state: "idle" }, { totalRepos: 1 }), false);

// The account line: the mode only when connected.
const rust = readFileSync(new URL("../../src-tauri/src/oauth.rs", import.meta.url), "utf8");
const modes = [...rust.matchAll(/=> "(app|oauth|token|none)",/g)].map((m) => m[1]);
assert.deepEqual([...new Set(modes)].sort(), ["app", "none", "oauth", "token"]);
for (const mode of ["app", "oauth", "token"]) {
  const key = githubModeKey("connected", mode);
  assert.ok(key && key in en, `${mode}: ${key}`);
}
assert.equal(githubModeKey("connected", "app"), "set.ghModeApp");
assert.equal(githubModeKey("connected", "none"), null);
assert.equal(githubModeKey("connected", undefined), null);
assert.equal(githubModeKey("reconnect", "app"), null);
assert.equal(githubModeKey("none", "oauth"), null);

// Every new text exists, and the tile and Settings use them.
for (const key of ["int.ghSummary", "int.ghDraft", "int.ghNeedsApp", "set.ghInstall", "set.ghInstallHint"]) {
  assert.ok(key in en, `en.ts has no ${key}`);
}
const tile = readFileSync(new URL("../../src/views/integrations.ts", import.meta.url), "utf8");
assert.match(tile, /Bridge\.openGithub\(/, "rows open through Rust's github.com check");
const settings = readFileSync(new URL("../../src/settings/main.ts", import.meta.url), "utf8");
assert.match(settings, /Bridge\.openGithubInstall\(\)/);

console.log("github-tile-check: all passed");

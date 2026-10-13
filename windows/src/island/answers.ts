// Claude Code questions (AskUserQuestion) on the island card, and which clicks a permission or
// question card takes: the pure part, with no DOM, no Tauri and no State, so it can be checked on
// its own (spec §2.3, §6).
//
// Texts are kept exactly as the router sent them, possibly cut by the relay at 2000 bytes
// with "…": the answers are keyed by that question text and hold those labels, and the relay
// maps them back to the full texts. Anything it cannot map one to one, it does not answer.
// An "Other" text is sent as `{ other: text }`; the relay prints it as the plain answer.

export interface QuestionOption {
  label: string;
  /** "" when the option has none. */
  description: string;
}

export interface Question {
  question: string;
  /** The short chip Claude Code shows above the question; "" when there is none. */
  header: string;
  options: QuestionOption[];
  multiSelect: boolean;
}

/** Per question, in the same order, the indexes of the picked options. */
export type Picks = readonly (readonly number[])[];

/** Per question, in the same order, what was typed in its "Other" field ("" when nothing). */
export type Others = readonly string[];

/**
 * Question text → label, the labels of a multi-select (never empty), or `{ other: text }` for
 * what the user typed in "Other" (spec §6): Bridge.questionAnswer's shape.
 */
export type Answers = Record<string, string | [string, ...string[]] | { other: string }>;

/** The longest "Other" text, in characters (code points), as the relay and Rust count it. */
export const OTHER_MAX_CHARS = 4000;

const MAX_QUESTIONS = 4;
const MIN_OPTIONS = 2;
const MAX_OPTIONS = 4;

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** A string field that may be missing ("" then); any other type is a bad shape (null). */
function optionalText(v: unknown): string | null {
  if (v === undefined) return "";
  return typeof v === "string" ? v : null;
}

function parseOption(raw: unknown): QuestionOption | null {
  if (!isRecord(raw)) return null;
  const label = raw.label;
  if (typeof label !== "string" || label.trim() === "") return null;
  const description = optionalText(raw.description);
  if (description === null) return null;
  return { label, description };
}

function parseQuestion(raw: unknown): Question | null {
  if (!isRecord(raw)) return null;
  const question = raw.question;
  if (typeof question !== "string" || question.trim() === "") return null;
  const header = optionalText(raw.header);
  if (header === null) return null;
  const multi = raw.multiSelect;
  if (multi !== undefined && typeof multi !== "boolean") return null;
  const rawOptions = raw.options;
  if (!Array.isArray(rawOptions) || rawOptions.length < MIN_OPTIONS || rawOptions.length > MAX_OPTIONS) {
    return null;
  }
  const options: QuestionOption[] = [];
  for (const o of rawOptions) {
    const option = parseOption(o);
    // Two equal labels: the relay could not tell which one was meant.
    if (!option || options.some((x) => x.label === option.label)) return null;
    options.push(option);
  }
  return { question, header, options, multiSelect: multi === true };
}

/**
 * `tool_input` of an AskUserQuestion: 1 to 4 questions, each with a text and 2 to 4 options
 * with distinct labels, and no two questions with the same text. Anything else is null: the
 * card cannot answer it, so the agent asks in its own window.
 */
export function parseQuestions(toolInput: unknown): Question[] | null {
  if (!isRecord(toolInput)) return null;
  const raw = toolInput.questions;
  if (!Array.isArray(raw) || raw.length < 1 || raw.length > MAX_QUESTIONS) return null;
  const questions: Question[] = [];
  for (const r of raw) {
    const q = parseQuestion(r);
    if (!q || questions.some((x) => x.question === q.question)) return null;
    questions.push(q);
  }
  return questions;
}

/**
 * The `hook-withdraw` event (Rust took a question back: its time ran out or its server stopped)
 * takes down the card on screen only when it is the card of that request. A request that is not
 * the card (answered since, never shown, or one the island let go of itself) is left alone.
 */
export function withdrawsCard(cardRequestId: string | null | undefined, requestId: string): boolean {
  return requestId !== "" && cardRequestId === requestId;
}

/** One single-select question: a click on an option is the answer, with no Answer button. */
export function answersOnClick(questions: readonly Question[]): boolean {
  return questions.length === 1 && !questions[0].multiSelect;
}

/**
 * The picks of one question after a click on option `index`: single-select takes it in place of
 * the old pick, multi-select toggles it. Kept in option order; an index outside changes nothing.
 */
export function togglePick(question: Question, current: readonly number[], index: number): number[] {
  if (!Number.isInteger(index) || index < 0 || index >= question.options.length) return [...current];
  if (!question.multiSelect) return [index];
  const next = current.includes(index) ? current.filter((i) => i !== index) : [...current, index];
  return next.sort((a, b) => a - b);
}

/**
 * The "Other" text to send: `typed` trimmed, or null when that is empty or longer than
 * OTHER_MAX_CHARS characters (then it is no answer).
 */
export function otherText(typed: string | undefined): string | null {
  const text = (typed ?? "").trim();
  if (text === "" || [...text].length > OTHER_MAX_CHARS) return null;
  return text;
}

/**
 * Question `i` is answered: by its "Other" text, or else by its picks (exactly one for
 * single-select, at least one for multi-select, all distinct and valid).
 */
export function questionAnswered(q: Question, picks: readonly number[] | undefined, other?: string): boolean {
  if (otherText(other) !== null) return true;
  const p = picks ?? [];
  if (p.length === 0 || (!q.multiSelect && p.length !== 1)) return false;
  if (new Set(p).size !== p.length) return false;
  return p.every((x) => Number.isInteger(x) && x >= 0 && x < q.options.length);
}

/** Every question is answered (questionAnswered), with picks given for each. */
export function canAnswer(questions: readonly Question[], picks: Picks, others: Others = []): boolean {
  if (questions.length === 0 || picks.length !== questions.length) return false;
  return questions.every((q, i) => questionAnswered(q, picks[i], others[i]));
}

/** How long a card's buttons ignore clicks once they are built: a double click must not answer the next card. */
export const CLICK_GUARD_MS = 400;

/** The card a button was built for, and when (performance.now() ms). */
export interface CardBinding {
  requestId: string;
  builtAt: number;
}

/**
 * A click on a card's button counts only on the card it was built for, still the pending one,
 * and only from CLICK_GUARD_MS after the buttons were built. Any other click is ignored.
 */
export function clickCounts(bound: CardBinding, pendingId: string | null | undefined, now: number): boolean {
  return pendingId != null && bound.requestId === pendingId && now - bound.builtAt >= CLICK_GUARD_MS;
}

/**
 * Question text → `{ other: text }` when its "Other" text is an answer (the typed text overrides
 * the picks), else the picked label, or the picked labels in option order for a multi-select
 * (an array even with one pick). Only for what `canAnswer` takes: anything else throws, so a
 * half answer is never sent.
 */
export function buildAnswers(questions: readonly Question[], picks: Picks, others: Others = []): Answers {
  if (!canAnswer(questions, picks, others)) throw new RangeError("picks do not answer every question");
  const answers: Answers = {};
  questions.forEach((q, i) => {
    const other = otherText(others[i]);
    if (other !== null) {
      answers[q.question] = { other };
      return;
    }
    // canAnswer holds at least one pick per question.
    const labels = [...picks[i]].sort((a, b) => a - b).map((x) => q.options[x].label) as [string, ...string[]];
    answers[q.question] = q.multiSelect ? labels : labels[0];
  });
  return answers;
}

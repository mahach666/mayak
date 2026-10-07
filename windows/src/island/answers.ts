// Claude Code questions (AskUserQuestion) on the island card: the pure part, with no DOM, no
// Tauri and no State, so it can be checked on its own (spec §2.3).
//
// Texts are kept exactly as the router sent them, possibly cut by the relay at 2000 bytes
// with "…": the answers are keyed by that question text and hold those labels, and the relay
// maps them back to the full texts. Anything it cannot map one to one, it does not answer.

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

/** Question text → label, or the labels of a multi-select (never empty): Bridge.questionAnswer's shape. */
export type Answers = Record<string, string | [string, ...string[]]>;

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

/** Every question has its picks: exactly one for single-select, at least one for multi-select, all distinct and valid. */
export function canAnswer(questions: readonly Question[], picks: Picks): boolean {
  if (questions.length === 0 || picks.length !== questions.length) return false;
  return questions.every((q, i) => {
    const p = picks[i];
    if (p.length === 0 || (!q.multiSelect && p.length !== 1)) return false;
    if (new Set(p).size !== p.length) return false;
    return p.every((x) => Number.isInteger(x) && x >= 0 && x < q.options.length);
  });
}

/**
 * Question text → the picked label, or the picked labels in option order for a multi-select
 * (an array even with one pick). Only for picks `canAnswer` takes: anything else throws, so a
 * half answer is never sent.
 */
export function buildAnswers(questions: readonly Question[], picks: Picks): Answers {
  if (!canAnswer(questions, picks)) throw new RangeError("picks do not answer every question");
  const answers: Answers = {};
  questions.forEach((q, i) => {
    // canAnswer holds at least one pick per question.
    const labels = [...picks[i]].sort((a, b) => a - b).map((x) => q.options[x].label) as [string, ...string[]];
    answers[q.question] = q.multiSelect ? labels : labels[0];
  });
  return answers;
}

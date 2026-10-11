// Small typing helpers for query fields and the document editor: closes
// brackets and quotes, completes helper names (`Obj` -> `ObjectId("|")`), and
// wraps pasted or typed ids/dates in their shell helpers. Edits go through execCommand so the native undo stack keeps working.

type Field = HTMLInputElement | HTMLTextAreaElement;

const PAIRS: Record<string, string> = { "{": "}", "[": "]", "(": ")", '"': '"', "'": "'" };
const CLOSERS = new Set(["}", "]", ")"]);
/** Keys after which a bare 24-hex token is taken as finished and wrapped. */
const TOKEN_ENDS = new Set([",", "}", "]", " ", "Enter"]);

/** Helpers completed from a typed prefix, with what goes around the caret. */
const HELPERS: [name: string, open: string, close: string][] = [
  ["ObjectId", '("', '")'],
  ["ISODate", '("', '")'],
  ["UUID", '("', '")'],
  ["NumberDecimal", '("', '")'],
  ["NumberLong", "(", ")"],
  ["NumberInt", "(", ")"],
  ["Timestamp", "(", ")"],
];
const MIN_PREFIX = 3;

/**
 * After a completion, the rest of the name and its opening (`ectId("`) that
 * the user may keep typing out of habit; those keys are swallowed.
 */
const pending = new WeakMap<Field, { pos: number; rest: string }>();

const HEX24 = /^[0-9a-f]{24}$/i;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const ISO_DATE = /^\d{4}-\d{2}-\d{2}([T ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})?)?$/;

/** Shell form of a pasted literal, or null if it should stay as typed. */
export function wrapLiteral(text: string): string | null {
  if (HEX24.test(text)) return `ObjectId("${text}")`;
  if (UUID.test(text)) return `UUID("${text}")`;
  if (ISO_DATE.test(text)) return `ISODate("${text}")`;
  return null;
}

/** Whether `pos` falls inside a quoted string. */
export function inString(text: string, pos: number): boolean {
  let quote: string | null = null;
  for (let i = 0; i < pos; i++) {
    const c = text[i];
    if (quote) {
      if (c === "\\") i++;
      else if (c === quote) quote = null;
    } else if (c === '"' || c === "'") {
      quote = c;
    }
  }
  return quote !== null;
}

/** The innermost bracket still open at `pos`, ignoring strings. */
function enclosingOpener(text: string, pos: number): string | null {
  const stack: string[] = [];
  let quote: string | null = null;
  for (let i = 0; i < pos; i++) {
    const c = text[i];
    if (quote) {
      if (c === "\\") i++;
      else if (c === quote) quote = null;
    } else if (c === '"' || c === "'") quote = c;
    else if (c === "{" || c === "[" || c === "(") stack.push(c);
    else if (c === "}" || c === "]" || c === ")") stack.pop();
  }
  return stack.at(-1) ?? null;
}

/** Whether `pos` is where a value goes (after `:`, or inside an array or call), not a key. */
function isValuePosition(text: string, pos: number): boolean {
  const prev = text.slice(0, pos).trimEnd().at(-1);
  if (prev === ":" || prev === "[" || prev === "(") return true;
  return prev === "," && enclosingOpener(text, pos) !== "{";
}

/** The helper a typed prefix stands for, if exactly one matches. */
export function completeHelper(prefix: string): (typeof HELPERS)[number] | null {
  if (prefix.length < MIN_PREFIX) return null;
  const p = prefix.toLowerCase();
  const matches = HELPERS.filter(([name]) => name.toLowerCase().startsWith(p));
  return matches.length === 1 ? matches[0] : null;
}

/** Replaces the selection with `text` and puts the caret `caret` chars into it. */
export function insertText(el: Field, text: string, caret = text.length) {
  const start = el.selectionStart ?? 0;
  if (!document.execCommand("insertText", false, text)) {
    el.setRangeText(text, start, el.selectionEnd ?? start, "end");
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }
  el.setSelectionRange(start + caret, start + caret);
}

function replaceRange(el: Field, from: number, to: number, text: string) {
  el.setSelectionRange(from, to);
  insertText(el, text);
}

/** Wraps a finished bare ObjectId hex right before the caret. */
function wrapTokenBeforeCaret(el: Field) {
  const pos = el.selectionStart ?? 0;
  const before = el.value.slice(0, pos);
  const m = before.match(/(^|[^\w"'])([0-9a-f]{24})$/i);
  if (!m || inString(el.value, pos)) return;
  replaceRange(el, pos - 24, pos, `ObjectId("${m[2]}")`);
}

function onKeydown(e: KeyboardEvent) {
  const key = e.key;
  if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.isComposing) return;
  // On many layouts (Turkish included) Option types `{ } [ ]`; only real
  // shortcuts like Option+Backspace are left to the browser.
  if (e.altKey && key.length !== 1) return;
  const el = e.currentTarget as Field;

  const p = pending.get(el);
  if (p) {
    if (el.selectionStart === p.pos && el.selectionEnd === p.pos && key.toLowerCase() === p.rest[0].toLowerCase()) {
      e.preventDefault();
      p.rest = p.rest.slice(1);
      if (!p.rest) pending.delete(el);
      return;
    }
    pending.delete(el);
  }

  if (TOKEN_ENDS.has(key) && el.selectionStart === el.selectionEnd) wrapTokenBeforeCaret(el);

  const v = el.value;
  const start = el.selectionStart ?? 0;
  const end = el.selectionEnd ?? 0;
  const prev = v[start - 1] ?? "";
  const next = v[end] ?? "";
  const quoted = inString(v, start);

  // Typing a closer that is already there just steps over it.
  if (start === end && next === key && (CLOSERS.has(key) || (PAIRS[key] === key && quoted))) {
    e.preventDefault();
    el.setSelectionRange(start + 1, start + 1);
    return;
  }

  if (key in PAIRS) {
    const close = PAIRS[key];
    if (start !== end) {
      // Wrap the selection, keeping it selected.
      e.preventDefault();
      insertText(el, key + v.slice(start, end) + close);
      el.setSelectionRange(start + 1, end + 1);
      return;
    }
    if (quoted) return;
    if (key === close && /[\w\\]/.test(prev)) return;
    if (next && !/[\s}\]),:]/.test(next)) return;
    e.preventDefault();
    insertText(el, key + close, 1);
    return;
  }

  if (key === "Backspace" && start === end && PAIRS[prev] && PAIRS[prev] === next) {
    e.preventDefault();
    replaceRange(el, start - 1, start + 1, "");
    return;
  }

  if (key === "Enter" && el instanceof HTMLTextAreaElement && !e.shiftKey) {
    e.preventDefault();
    const lineStart = v.lastIndexOf("\n", start - 1) + 1;
    const indent = v.slice(lineStart).match(/^[ \t]*/)![0];
    const opens = prev === "{" || prev === "[";
    if (opens && next === PAIRS[prev]) {
      insertText(el, `\n${indent}  \n${indent}`, indent.length + 3);
    } else {
      insertText(el, `\n${indent}${opens ? "  " : ""}`);
    }
  }
}

function onInput(e: Event) {
  // Only fresh typing: undo, delete and paste must not re-trigger completion.
  if ((e as InputEvent).inputType !== "insertText") return;
  const el = e.currentTarget as Field;
  const pos = el.selectionStart ?? 0;
  if (pos !== el.selectionEnd || /\w/.test(el.value[pos] ?? "")) return;
  const m = el.value.slice(0, pos).match(/(^|[^\w$"'.])([A-Za-z]+)$/);
  if (!m) return;
  const start = pos - m[2].length;
  if (inString(el.value, start) || !isValuePosition(el.value, start)) return;
  const helper = completeHelper(m[2]);
  if (!helper) return;
  const [name, open, close] = helper;
  const caret = start + name.length + open.length;
  replaceRange(el, start, pos, name + open + close);
  el.setSelectionRange(caret, caret);
  pending.set(el, { pos: caret, rest: name.slice(m[2].length) + open });
}

function onPaste(e: ClipboardEvent) {
  const el = e.currentTarget as Field;
  const text = e.clipboardData?.getData("text/plain").trim() ?? "";
  if (inString(el.value, el.selectionStart ?? 0)) return;
  const wrapped = wrapLiteral(text);
  if (!wrapped) return;
  e.preventDefault();
  insertText(el, wrapped);
}

/** Svelte action: `<input use:smartInput>`. */
export function smartInput(field: Field) {
  // Widened so TypeScript picks the typed overloads for the union.
  const el: HTMLElement = field;
  el.addEventListener("keydown", onKeydown);
  el.addEventListener("input", onInput);
  el.addEventListener("paste", onPaste);
  return {
    destroy() {
      el.removeEventListener("keydown", onKeydown);
      el.removeEventListener("input", onInput);
      el.removeEventListener("paste", onPaste);
    },
  };
}

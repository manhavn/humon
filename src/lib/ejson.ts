// Turns canonical extended JSON from the backend into something displayable,
// and back into shell syntax that the query parser accepts.

export type Leaf = {
  kind:
    | "string"
    | "int"
    | "long"
    | "double"
    | "decimal"
    | "bool"
    | "null"
    | "oid"
    | "date"
    | "uuid"
    | "binary"
    | "regex"
    | "timestamp"
    | "other";
  /** Short form shown in the tree. */
  text: string;
  /** Form that pastes back into a filter. */
  shell: string;
};

export type Node =
  | { kind: "object"; entries: [string, unknown][] }
  | { kind: "array"; items: unknown[] }
  | Leaf;

type Obj = Record<string, unknown>;

const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null && !Array.isArray(v);
const quote = (s: string) => JSON.stringify(s);

function base64ToHex(b64: string): string {
  return Array.from(atob(b64), (c) => c.charCodeAt(0).toString(16).padStart(2, "0")).join("");
}

function dateText(ms: string): string {
  const d = new Date(Number(ms));
  return Number.isNaN(d.getTime()) ? ms : d.toISOString();
}

/** Recognises the single-key wrappers extended JSON uses for typed values. */
function typed(v: Obj): Leaf | null {
  const keys = Object.keys(v);
  if (keys.length === 0 || !keys[0].startsWith("$")) return null;
  const k = keys[0];
  const x = v[k] as never;
  switch (k) {
    case "$oid":
      return { kind: "oid", text: x, shell: `ObjectId(${quote(x)})` };
    case "$date": {
      const ms = isObj(x) ? String((x as Obj).$numberLong) : String(x);
      const iso = dateText(ms);
      return { kind: "date", text: iso, shell: `ISODate(${quote(iso)})` };
    }
    case "$numberInt":
      return { kind: "int", text: x, shell: x };
    case "$numberLong":
      return { kind: "long", text: x, shell: `NumberLong(${quote(x)})` };
    case "$numberDouble": {
      const shell = x === "Infinity" || x === "-Infinity" || x === "NaN" ? x : String(Number(x));
      return { kind: "double", text: x, shell: /^-?\d+$/.test(shell) ? `${shell}.0` : shell };
    }
    case "$numberDecimal":
      return { kind: "decimal", text: x, shell: `NumberDecimal(${quote(x)})` };
    case "$binary": {
      const { base64, subType } = x as { base64: string; subType: string };
      if (subType === "04") {
        const h = base64ToHex(base64);
        const uuid = `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
        return { kind: "uuid", text: uuid, shell: `UUID(${quote(uuid)})` };
      }
      const sub = parseInt(subType, 16);
      return { kind: "binary", text: `Binary(${sub}, ${base64.length > 40 ? base64.slice(0, 40) + "…" : base64})`, shell: `BinData(${sub}, ${quote(base64)})` };
    }
    case "$regularExpression": {
      const { pattern, options } = x as { pattern: string; options: string };
      const text = `/${pattern}/${options}`;
      return { kind: "regex", text, shell: text };
    }
    case "$timestamp": {
      const { t, i } = x as { t: number; i: number };
      return { kind: "timestamp", text: `Timestamp(${t}, ${i})`, shell: `Timestamp(${t}, ${i})` };
    }
    case "$minKey":
      return { kind: "other", text: "MinKey", shell: "MinKey()" };
    case "$maxKey":
      return { kind: "other", text: "MaxKey", shell: "MaxKey()" };
    case "$undefined":
      return { kind: "null", text: "undefined", shell: "undefined" };
    case "$symbol":
      return { kind: "string", text: x, shell: quote(x) };
    case "$code":
      return { kind: "other", text: `Code(${x})`, shell: quote(x) };
    default:
      return null;
  }
}

export function classify(v: unknown): Node {
  if (v === null) return { kind: "null", text: "null", shell: "null" };
  if (typeof v === "string") return { kind: "string", text: v, shell: quote(v) };
  if (typeof v === "boolean") return { kind: "bool", text: String(v), shell: String(v) };
  if (typeof v === "number") return { kind: "double", text: String(v), shell: String(v) };
  if (Array.isArray(v)) return { kind: "array", items: v };
  if (isObj(v)) return typed(v) ?? { kind: "object", entries: Object.entries(v) };
  return { kind: "other", text: String(v), shell: String(v) };
}

const IDENT = /^[A-Za-z_$][\w$]*$/;

/** Pretty-prints a value in mongo shell syntax. */
export function toShell(v: unknown, indent = ""): string {
  const node = classify(v);
  const inner = indent + "  ";
  if (node.kind === "object") {
    if (node.entries.length === 0) return "{}";
    const lines = node.entries.map(
      ([k, val]) => `${inner}${IDENT.test(k) ? k : quote(k)}: ${toShell(val, inner)}`,
    );
    return `{\n${lines.join(",\n")}\n${indent}}`;
  }
  if (node.kind === "array") {
    if (node.items.length === 0) return "[]";
    return `[\n${node.items.map((x) => inner + toShell(x, inner)).join(",\n")}\n${indent}]`;
  }
  return node.shell;
}

/** One-line preview of a collapsed object or array. */
export function preview(node: Node): string {
  if (node.kind === "object") {
    const n = node.entries.length;
    return n === 0 ? "{}" : `{ ${n} alan }`;
  }
  if (node.kind === "array") return `Array [${node.items.length}]`;
  return node.text;
}

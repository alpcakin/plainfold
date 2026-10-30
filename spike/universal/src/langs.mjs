// The only language-specific part of the walker: which tree-sitter node types mean what.
// Adding a language = adding one entry here (plus a language server in servers.mjs). The walker never names a language.
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const wasm = (pkg, file) => path.join(here, "..", "node_modules", pkg, file);

const tsCommon = {
  // bodies that run when called (their calls belong to them, not to the enclosing function)
  functions: ["function_declaration", "generator_function_declaration", "function_expression", "arrow_function", "method_definition"],
  functionName: (n) => n.childForFieldName("name"),
  // a call: node type → field holding what is called
  calls: { call_expression: "function", new_expression: "constructor" },
  // things that are not calls in the syntax but invoke something (a component used as an element)
  elements: { jsx_self_closing_element: "name", jsx_opening_element: "name" },
  // what to look through to find the name being called
  unwrap: ["parenthesized_expression", "non_null_expression", "await_expression", "as_expression", "satisfies_expression"],
  member: { member_expression: "property" },
  names: ["identifier", "property_identifier", "type_identifier", "shorthand_property_identifier"],
  // control flow: frames in the diagram
  branches: {
    if_statement: { cond: "condition", then: "consequence", else: "alternative" },
    ternary_expression: { cond: "condition", then: "consequence", else: "alternative" },
    switch_statement: { cond: "value", then: "body" },
    try_statement: { then: "body", else: "handler", finally: "finalizer" },
  },
  loops: { for_statement: "body", for_in_statement: "body", while_statement: "body", do_statement: "body" },
  // following a value one more step after go-to-definition lands on a local name
  follow: {
    declarator: { type: "variable_declarator", name: "name", value: "value" },
    destructure: { pattern: "object_pattern", pair: { type: "pair_pattern", key: "key", value: "value" }, shorthand: "shorthand_property_identifier_pattern" },
    returns: "return_statement",
    object: { type: "object", pair: { type: "pair", key: "key", value: "value" }, shorthand: "shorthand_property_identifier" },
    parameter: ["required_parameter", "optional_parameter", "formal_parameters"],
  },
};

export const LANGS = {
  typescript: { ...tsCommon, exts: [".ts", ".mts", ".cts"], grammar: wasm("tree-sitter-typescript", "tree-sitter-typescript.wasm"), server: "typescript", lspId: "typescript" },
  tsx: { ...tsCommon, exts: [".tsx", ".jsx", ".js", ".mjs"], grammar: wasm("tree-sitter-typescript", "tree-sitter-tsx.wasm"), server: "typescript", lspId: "typescriptreact" },
  python: {
    exts: [".py"],
    grammar: wasm("tree-sitter-python", "tree-sitter-python.wasm"),
    server: "python",
    lspId: "python",
    functions: ["function_definition", "lambda"],
    functionName: (n) => n.childForFieldName("name"),
    calls: { call: "function" },
    elements: {},
    unwrap: ["parenthesized_expression", "await"],
    member: { attribute: "attribute" },
    names: ["identifier"],
    branches: {
      if_statement: { cond: "condition", then: "consequence", else: "alternative" },
      elif_clause: { cond: "condition", then: "consequence" },
      else_clause: { then: "body" },
      conditional_expression: {},
      try_statement: { then: "body" },
      match_statement: { cond: "subject", then: "body" },
    },
    loops: { for_statement: "body", while_statement: "body", list_comprehension: "body", dictionary_comprehension: "body", set_comprehension: "body", generator_expression: "body" },
    follow: {
      declarator: { type: "assignment", name: "left", value: "right" },
      destructure: null,
      returns: "return_statement",
      object: null,
      parameter: ["parameters", "typed_parameter", "default_parameter", "typed_default_parameter"],
    },
  },
};

export function langFor(file) {
  const ext = path.extname(file);
  return Object.entries(LANGS).find(([, l]) => l.exts.includes(ext))?.[0] ?? null;
}

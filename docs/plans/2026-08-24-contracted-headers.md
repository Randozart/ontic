# Contracted Headers Stage (.hpp membrane)

**Date:** 2026-08-24
**Author:** Randy + agent
**Status:** Approved (standing queue; C++26 conditions from strategy report §4)

## 1. Scope

Emit `<name>-<key8>.hpp` beside every `.h`: C++ declarations carrying sieve
provenance as native C++26 contracts where translatable, portable
documentation otherwise. Consumers — including LLM-written glue — inherit
kernel preconditions mechanically at the call site.

## 2. Translation rules (v1 subset)

| Sketch | C++ contract text |
|---|---|
| `%scalar` | parameter name |
| `len(%list)` | `<list>_s` (Flat-MemRef size field) |
| `+ - * / %`, comparisons, parens, int/float literals | verbatim |
| anything else (until refs, res postconditions, unknown fns) | skipped; listed in a trailing comment |

Both forms emitted: `#ifdef ONTIC_CONTRACTS` native `pre(...)` declarations;
plain declarations plus `// ontic requires:` lines otherwise. Metadata block
(`// ontic: pre [ ... ]`) always present so tooling can parse without a
compiler.

## 3. Work items

| # | Item | Gate |
|---|------|------|
| H1 | `lower::emit_header_hpp` + translation fn | golden snapshot tests (matvec len-relations; gauss scalar bound) |
| H2 | Wire into `emit_and_store` artifacts + manifest | solve produces .hpp; manifest records it |
| H3 | Compile smoke: clang++ -std=c++26 parses contracted decl behind macro; -std=c++17 parses fallback | LG-H: both modes compile |
| H4 | CHANGES + report | FG-H |

## 4. Non-goals

Postcondition `res` translation (needs return-value naming per ABI — later),
contract violation handlers (consumer-side), optimizer/tuples/genrand items.

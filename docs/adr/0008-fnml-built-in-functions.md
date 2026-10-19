# 0008. RML-FNML: function descriptions and code together in Rust, plus host functions

Status: accepted · 2026-10-09

## Context

RML-FNML calls functions by IRI (`rml:function grel:toUpperCase`). The set of function IRIs is open, so an engine needs, for every function it can run, both a description (parameters and outputs) and an implementation. The specification requires that a call to an unknown function, or with a parameter or output the function does not have, produces nothing (test cases 0102-0104). The 20 test cases use eight functions (five GREL, three IDLab); the FnO descriptions shipped with the tests cover only some of the GREL ones.

## Options

| Option | For | Against |
|---|---|---|
| Read FnO description documents at run time, look up code by IRI | Descriptions identical to the published ones | Code is needed anyway; the IDLab functions in the tests have no description document; where to fetch documents from |
| **Descriptions and code together in Rust, plus a host API** | Description and code cannot disagree; no network or files; calls are checked before any data is read | Adding a built-in function means changing code |
| External execution (scripts, WebAssembly plug-ins, processes) | Anything can be called | Against the pure scope (ADR 0001); reproducibility and security |
| Port the whole GREL and IDLab libraries | Same functions as other engines | Hundreds of functions whose behaviour no test pins down |

## Decision

- `rmlrs_engine::functions::Function { iri, parameters, outputs, run }`, behind the `fnml` feature (on in the CLI and the conformance runner).
- Built in: only the eight functions the test cases use. An output carries its natural datatype (`string_length` gives `xsd:integer`).
- Callers add functions through `Options::functions`; they take precedence over built-in ones.
- Every call is checked against its function's description when the mapping is compiled: unknown functions, parameters or outputs, and missing parameters are mapping errors.
- Multi-valued inputs give a Cartesian product; an input without a value means no call (as in templates). `rml:condition` keeps a term map's values only when the condition returns `true`.

## Consequences

- RML-FNML passes 19 of 20. RMLFNMLTC0101 expects an error for a missing CSV column, which the RML-IO-Registry text says is NULL; it is listed in `conformance/known-issues.csv`, like RMLIOREGTC0001b.
- A GREL function RMLrs does not implement is a mapping error, not "not supported": function IRIs are open-ended and the specification defines this case as an error.
- The condition functions the specification lists (isNull, isNotNull, equals, notEquals, IF) have no IRIs yet; only `idlab-fn:equal`, which a test uses, is built in.

## Revisit when

- Real mappings need many more functions: then a separate crate, possibly with generated FnO descriptions.
- Host functions need state: then a boxed closure instead of a function pointer.

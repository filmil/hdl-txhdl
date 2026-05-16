# Proposal and Next Steps

## Findings
The user requested to read `GEMINI.md` to orient, however, `GEMINI.md` does not exist in the repository. Instead, I explored the project by reading the specifications in `filmil/` and `spec/`.

The project aims to create a new, modern Hardware Description Language (referred to as FLHDL, LHdl, and TxHDL across different documents) designed to be transaction-centric and easily parsed (LL(1) deterministic grammar).

## Proposed Next Steps

1. **Consolidate Specifications and Naming**
   - The specifications exist in multiple files (`filmil/workspace/spec.md`, `filmil/workspace/lhdl-proposal.md`, `spec/language.md`) and use different names for the language (FLHDL, LHdl, TxHDL).
   - **Action:** Decide on a single name for the language and merge the documents into a single, unified language specification (`spec/language.md` or a similar canonical file).

2. **Implement Lexer and Parser**
   - The language is explicitly designed with an LL(1) deterministic grammar to allow for fast, unambiguous parsing.
   - **Action:** Begin implementing the Lexer and Parser in the chosen host language (Go or Rust are suggested in the specs). Define the tokens and write a basic recursive-descent parser based on the proposed EBNF grammar.

3. **Define the Abstract Syntax Tree (AST)**
   - Before full compilation or simulation can occur, the parsed tokens need to be structured.
   - **Action:** Define the AST nodes corresponding to the language constructs (Interfaces, Modules, Pipelines, Tags, etc.) in the host language.

4. **Develop a Skeleton Compiler/Toolchain**
   - **Action:** Create a basic CLI tool (e.g., `flhdlc`) that can take a source file, parse it into an AST, and perform basic semantic checks (like undefined variables or type mismatches).

5. **Establish Testing Infrastructure**
   - Since the spec mentions a focus on FFI and Co-Simulation via RPC (gRPC) rather than traditional testbenches, the testing strategy should be defined early.
   - **Action:** Set up the basic infrastructure to parse an FLHDL file and generate the corresponding C++/Rust/Go structs for FFI interactions as described in the testing facet of the spec.

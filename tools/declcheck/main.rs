// SPDX-License-Identifier: Apache-2.0
//! Is every name a generated netlist uses declared before its use?
//!
//! Verilog makes a net of a name first met in a port connection or on
//! the left of an `assign`, and a later `wire` of the same name is then
//! a second declaration. Synthesis, Verilator and Yosys accept that in
//! silence; Vivado's simulator refuses it, and every Vivado target in
//! the tree is manual. So a netlist could break the board simulation
//! behind a green suite, and one did: hart 1's tied inputs were joined
//! to its instance three thousand lines before their wires (#1575).
//!
//! This reads each module in order, as xvlog does, and fails on every
//! name used before the module declares it: a port, a `wire`, a `reg`,
//! an `integer`, a parameter or an instance. A module's type, a port
//! named after a `.`, a number's base, a system task and a directive
//! are not uses.
//!
//! It fails as well on an unsized decimal that is an operand of a
//! concatenation, which Verilog-2005 does not allow and a tool that
//! takes it reads as 32 bits wide. Verilator and synthesis say nothing
//! of that either, and a constant put in a field of a struct literal
//! lowered that way, `{5, inp_data}` (#1574).
//!
//! Usage:
//!
//! ```text
//! declcheck NETLIST.v ...
//! ```

use std::collections::BTreeSet;
use std::fs;
use std::process::ExitCode;

/// A token and the line it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tok {
    pub text: String,
    pub line: usize,
}

/// The text as names, numbers and single punctuation marks, with its
/// comments and strings left out; `(*` and `*)` are one token each.
pub fn tokens(src: &str) -> Vec<Tok> {
    let c: Vec<char> = src.chars().collect();
    let mut t = Vec::new();
    let mut line = 1;
    let mut i = 0;
    let push = |t: &mut Vec<Tok>, s: String, line: usize| {
        t.push(Tok { text: s, line })
    };
    while i < c.len() {
        let ch = c[i];
        let next = c.get(i + 1).copied();
        if ch == '\n' {
            line += 1;
            i += 1;
        } else if ch.is_whitespace() {
            i += 1;
        } else if ch == '/' && next == Some('/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && next == Some('*') {
            i += 2;
            while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                if c[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 2;
        } else if ch == '"' {
            i += 1;
            while i < c.len() && c[i] != '"' {
                i += 1;
            }
            i += 1;
        } else if ch == '(' && next == Some('*') && c.get(i + 2) != Some(&')') {
            push(&mut t, "(*".into(), line);
            i += 2;
        } else if ch == '*' && next == Some(')') {
            push(&mut t, "*)".into(), line);
            i += 2;
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            let st = i;
            while i < c.len()
                && (c[i].is_ascii_alphanumeric() || c[i] == '_' || c[i] == '$')
            {
                i += 1;
            }
            push(&mut t, c[st..i].iter().collect(), line);
        } else {
            push(&mut t, ch.to_string(), line);
            i += 1;
        }
    }
    t
}

const KEYWORDS: &[&str] = &[
    "always",
    "and",
    "assert",
    "assign",
    "assume",
    "begin",
    "case",
    "casex",
    "casez",
    "cover",
    "default",
    "else",
    "end",
    "endcase",
    "endfunction",
    "endgenerate",
    "endmodule",
    "for",
    "function",
    "generate",
    "if",
    "initial",
    "inout",
    "input",
    "integer",
    "localparam",
    "module",
    "negedge",
    "not",
    "or",
    "output",
    "parameter",
    "posedge",
    "reg",
    "signed",
    "unsigned",
    "while",
    "wire",
];

/// The words that open a declaration: what follows them, outside a
/// range and before an `=`, is declared.
const DECLARE: &[&str] = &[
    "input",
    "output",
    "inout",
    "wire",
    "reg",
    "integer",
    "genvar",
    "parameter",
    "localparam",
];

fn is_name(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
}

/// One use of a name before its module declares it.
#[derive(Debug, PartialEq, Eq)]
pub struct Late {
    pub module: String,
    pub name: String,
    pub line: usize,
}

/// Every name used before its declaration, module by module.
pub fn check(src: &str) -> Vec<Late> {
    let t = tokens(src);
    let mut late = Vec::new();
    let mut module = String::new();
    let mut declared: BTreeSet<String> = BTreeSet::new();
    // Inside a declaration, how deep in brackets and parentheses, and
    // how deep the declaration's own names are: one in a module's
    // header, nought in its body.
    let mut decl = false;
    let mut in_value = false;
    let mut depth = 0i32;
    let mut at = 0i32;
    let mut i = 0;
    while i < t.len() {
        let w = t[i].text.as_str();
        let next = t.get(i + 1).map(|x| x.text.as_str()).unwrap_or("");
        match w {
            "module" => {
                module = next.to_string();
                declared.clear();
                decl = false;
                depth = 0;
                i += 2;
                continue;
            }
            // An attribute, a directive, a system task, a number's
            // base, and a port named in a connection: none is a use.
            "(*" => {
                while i < t.len() && t[i].text != "*)" {
                    i += 1;
                }
            }
            "`" => {
                i += 1;
                if matches!(next, "ifdef" | "ifndef" | "elsif") {
                    i += 1;
                }
            }
            "'" | "$" | "." => i += 1,
            // A block's label.
            ":" if i > 0 && t[i - 1].text == "begin" => i += 1,
            _ if DECLARE.contains(&w) => {
                if !decl {
                    at = depth;
                }
                decl = true;
                in_value = false;
            }
            "[" | "(" | "{" => depth += 1,
            "]" | "}" => depth -= 1,
            ")" => {
                depth -= 1;
                // The end of a module's header ends its last port.
                if depth < at {
                    decl = false;
                }
            }
            "=" if decl && depth == at => in_value = true,
            "," if decl && depth == at => in_value = false,
            ";" => {
                decl = false;
                in_value = false;
            }
            _ if KEYWORDS.contains(&w) || !is_name(w) => {}
            // A declared name, which a range or a value may not be.
            _ if decl && depth == at && !in_value => {
                declared.insert(w.to_string());
            }
            _ if declared.contains(w) => {}
            // An instance: the module's type, its parameters, then its
            // name, which declares it.
            _ if next == "#"
                || (is_name(next)
                    && t.get(i + 2).is_some_and(|x| x.text == "(")) =>
            {
                i += 1;
                if t[i].text == "#" {
                    let mut d = 0;
                    i += 1;
                    loop {
                        match t[i].text.as_str() {
                            "(" => d += 1,
                            ")" => d -= 1,
                            _ => {}
                        }
                        i += 1;
                        if d == 0 {
                            break;
                        }
                    }
                }
                declared.insert(t[i].text.clone());
            }
            _ => late.push(Late {
                module: module.clone(),
                name: w.to_string(),
                line: t[i].line,
            }),
        }
        i += 1;
    }
    late
}

/// One unsized number that is an operand of a concatenation.
#[derive(Debug, PartialEq, Eq)]
pub struct Unsized {
    pub number: String,
    pub line: usize,
}

/// Every decimal that is itself an operand of a concatenation: not
/// sized, as `4'b0101` is, not a replication count, as the `4` of
/// `{4{x}}` is, and not inside an index or a parenthesised expression,
/// whose sizing is another rule (#1574).
pub fn unsized_numbers(src: &str) -> Vec<Unsized> {
    let t = tokens(src);
    let mut found = Vec::new();
    // One entry per open brace: how deep in brackets and parentheses
    // the text is since that brace.
    let mut braces: Vec<i32> = Vec::new();
    for (i, tok) in t.iter().enumerate() {
        let w = tok.text.as_str();
        match w {
            "{" => braces.push(0),
            "}" => {
                braces.pop();
            }
            "[" | "(" => {
                if let Some(d) = braces.last_mut() {
                    *d += 1;
                }
            }
            "]" | ")" => {
                if let Some(d) = braces.last_mut() {
                    *d -= 1;
                }
            }
            _ if w.chars().all(|c| c.is_ascii_digit()) => {
                let prev = i.checked_sub(1).map(|j| t[j].text.as_str());
                let next = t.get(i + 1).map(|x| x.text.as_str());
                let operand = braces.last() == Some(&0);
                let sized = prev == Some("'") || next == Some("'");
                let count = next == Some("{");
                if operand && !sized && !count {
                    found.push(Unsized {
                        number: w.to_string(),
                        line: tok.line,
                    });
                }
            }
            _ => {}
        }
    }
    found
}

fn main() -> ExitCode {
    let mut bad = 0;
    for path in std::env::args().skip(1) {
        let src = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{path}: {e}");
                return ExitCode::FAILURE;
            }
        };
        for l in check(&src) {
            eprintln!(
                "{path}:{}: `{}` is used in `{}` before it is declared",
                l.line, l.name, l.module
            );
            bad += 1;
        }
        for u in unsized_numbers(&src) {
            eprintln!(
                "{path}:{}: `{}` is an unsized number in a concatenation",
                u.line, u.number
            );
            bad += 1;
        }
    }
    if bad > 0 {
        eprintln!(
            "{bad} names used before their declaration or unsized \
             numbers in a concatenation"
        );
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHILD: &str = "module child(input a, output [1:0] y);\n  \
                         assign y = {a, 1'b0};\nendmodule\n";

    #[test]
    fn a_tie_declared_after_its_instance_is_found() {
        let src = format!(
            "{CHILD}module top(output [1:0] y);\n  \
             child c(.a(t0), .y(y));\n  wire t0;\n  \
             assign t0 = 1'b0;\nendmodule\n"
        );
        assert_eq!(
            check(&src),
            vec![Late {
                module: "top".into(),
                name: "t0".into(),
                line: 5
            }]
        );
    }

    #[test]
    fn a_tie_declared_before_is_not() {
        let src = format!(
            "{CHILD}module top(output [1:0] y);\n  wire t0;\n  \
             child c(.a(t0), .y(y));\n  assign t0 = 1'b0;\nendmodule\n"
        );
        assert_eq!(check(&src), vec![]);
    }

    #[test]
    fn what_a_netlist_holds_is_read() {
        let src = "`timescale 1ns/1ps\n\
            // `let x` is the wire x_w.\n\
            module chan #(parameter W = 8)(input clk, input rst,\n  \
              input [W-1:0] d, output reg [W-1:0] q);\n  \
              (* ASYNC_REG = \"TRUE\" *) reg [7:0] r = 8'h0;\n  \
              reg m [0:3];\n  integer m_i;\n  \
              initial for (m_i = 0; m_i < 4; m_i = m_i + 1) m[m_i] = 0;\n  \
              wire [7:0] t = $signed(r) >>> 2;\n  \
              always @(posedge clk) begin : run\n    \
                if (rst) q <= 0; else q <= d;\n\
              `ifdef FORMAL\n    assert (q == q);\n`endif\n  \
              end\n  \
              chan #(.W(8)) inner(.clk(clk), .rst(rst), .d(t), .q());\n\
              endmodule\n";
        assert_eq!(check(src), vec![]);
    }

    #[test]
    fn a_name_never_declared_is_found() {
        let src = "module m(input a);\n  assign b = a;\nendmodule\n";
        assert_eq!(check(src)[0].name, "b");
    }

    #[test]
    fn each_module_declares_its_own() {
        let src = "module a(input x);\nendmodule\n\
                   module b(input y);\n  assign x = y;\nendmodule\n";
        assert_eq!(check(src)[0].module, "b");
    }

    #[test]
    fn the_constant_of_1574_is_found() {
        let src = "module m(input [7:0] inp_data, output [11:0] o);\n  \
                   assign o = {5, inp_data};\nendmodule\n";
        assert_eq!(
            unsized_numbers(src),
            vec![Unsized {
                number: "5".into(),
                line: 2
            }]
        );
    }

    #[test]
    fn a_sized_constant_is_not() {
        let src = "assign o = {4'b0101, inp_data, 8'h0, 3'd5};\n";
        assert_eq!(unsized_numbers(src), vec![]);
    }

    #[test]
    fn a_count_an_index_and_an_expression_are_not() {
        let src = "assign o = {{4{x[7]}}, x[3:0], (y + 1), y[2]};\n";
        assert_eq!(unsized_numbers(src), vec![]);
    }

    #[test]
    fn a_number_outside_any_concatenation_is_not() {
        let src = "assign o = x + 1;\nalways @(posedge clk) r <= 0;\n";
        assert_eq!(unsized_numbers(src), vec![]);
    }

    #[test]
    fn a_nested_concatenation_is_read() {
        let src = "assign o = {x, {0, y}};\n";
        assert_eq!(unsized_numbers(src)[0].number, "0");
    }
}

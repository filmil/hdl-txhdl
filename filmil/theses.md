---
draft: true

---
<!-- SPDX-License-Identifier: Apache-2.0 -->

# **Idea seed: Some theses for a modern hardware definition language**

May 3, 2026[Filip Filmar](mailto:filmil@gmail.com)

# **​1.​ Theses**

1. It would be nice if [the language](https://drive.google.com/file/d/1ODgHWazEVGgtS6dAJ5EH3pH4AqgXW-Pf/view?usp=drive_link) were simple to parse.  
2. It makes sense to create a new domain-specific language in 2026\.  
3. Reuse is important for modern hardware design.  
4. Ability to predeclare interfaces promotes reuse and should be present.  
5. Generics are important for reuse too.  
6. The distinction between combinatorial and sequential networks should not matter.  
7. Often used primitives, like pipelines and state machines, should be first class citizens.  
8. At the design language level, there should be no notion of combinatorial, vs. sequential logic.  
9. Possibly, there need to be multiple “facets” of a description language: a design language, an implementation language, a configuration language.  
10. It is more important for the language to support useful and often used primitives, than it is to be fully general.  
11. A realistic language requires interop with conventional languages.  
12. A realistic language requires interop with testing infrastructure.

# **​2.​ Details**

## ​2.1.​ It would be nice if the language were simple to parse

Makes for a simple parser and tooling build. I had Gemini build an [example](https://gemini.google.com/share/98c4298a13c5).

Side note, better to have \`{}\` than \`begin .. end\`, avoids having \`begin\` and \`end\` as reserved words.

Contrast: VHDL is notoriously difficult to parse, leading to few good tools for handling it.

## ​2.2.​ It makes sense to create a new domain-specific language in 2026

One direction for the development of high level synthesis tools is to make them valid programs in an existing programming language. This makes sense as you can then compile and run them as regular programs, which is nice.

This avoids having to build a new toolchain for a new language. However, in 2026, with the availability of LLM powered tools, it is not that outrageous to think

## 

## ​2.4.​ Reuse is important for modern hardware design

Modern hardware design means millions to billions to trillions of components. In such an environment, connecting individual wires is too low an abstraction level. You must be able to build ever larger blocks and compose them together, if you are to meet the demands of modern hardware.

One way we do this is by making reuse easy to happen. While HDLs make it easy to swap out implementations, more is needed. Here are some elements of reuse that I think are required:

* Processing Elements  
* Composable pipelines  
* Module interfaces  
* Procedures and functions  
* Composable state machines  
* Libraries and packages

## ​2.5.​ Ability to predeclare interfaces promotes reuse and should be present

Interfaces in HDL/HSL are much more important than in languages intended for describing software.

Every interface in HDL/HSL gets at least two implementations by default: one is the register-transfer-level implementation, which is useful for simulation and synthesis, and another is the behavioral level, which is useful for simulation. Depending on your needs, you may have more or less detailed functional simulation, as well as architectures tailor made to specific implementation fabrics.  
This in turn means that having a way to communicate only an interface of a module instead of the module and its implementation, is immediately useful. Contrast that to software where you try not to abstract an implementation into an interface until you have enough implementation to justify the complexity. In software you wait until you get there. In hardware you are already there.

## ​2.6.​ Generics are important for reuse too

Again one area where hardware has an immediate need. The Go language famously did without generics for many years.

In hardware you may have a 8-bit, or a 16-bit,or a 32-bit, or a 64-bit Wishbone bus. There is no reason to require the interface authors to define separate modules for these. HDLs usually have these on the ready, but implementations are spotty.

## ​2.7.​ Often used primitives, like pipelines and state machines, should be first class citizens

## ​2.8.​ At the design language level, there should be no notion of combinatorial, vs. sequential logic

It seems to me that the combinatorial and sequential network distinction exists in design languages as an artifact of how we learned digital design. Since flip-flops are the cornerstone of stateful design, it makes sense that they exist as a concept when looking at a concrete digital design.

However, I think their importance diminishes in HDL/HLS. If you ever had to retime a design so that it makes timing, you probably noticed that save for a few lucky corner cases, you usually have to redesign it altogether. This leads me to believe that: (a) in many cases it is possible to convert a combinatorial network into an equivalent sequential network that can operate at a higher clock rate. And (b) if this is possible, then it’s also automatable.

With this in mind I think it should be possible to design a HLS language which does not use combinatorial and sequential logic as first class concepts. Instead, the statefulness of a computational network should be derived from how a HLS description is mapped to the underlying digital technology.

## ​2.9.​ Possibly, there need to be multiple “facets” of a description language: a design language, an implementation language, a configuration language

## ​2.10.​ It is more important for the language to support useful and often used primitives, than it is to be fully general

Instead, use interop with existing languages.

## ​2.11.​ A realistic language requires interop with conventional languages

To reuse already existing modules. Not sure how to make that happen.

## ​2.12.​ A realistic language requires interop with testing infrastructure

I think it might be more interesting to provide interop with existing languages (e.g. for testing or for simulation, or verification), than to add a non-synthesizable language subset which e.g. deals with file IO and such.  

<!--stackedit_data:
eyJoaXN0b3J5IjpbLTE5Mzk0ODY3MThdfQ==
-->
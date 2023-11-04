/*
 * Copyright (c) 2017-2020 Boucher, Antoni <bouanto@zoho.com>
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy of
 * this software and associated documentation files (the "Software"), to deal in
 * the Software without restriction, including without limitation the rights to
 * use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
 * the Software, and to permit persons to whom the Software is furnished to do so,
 * subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
 * FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
 * COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
 * IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
 * CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
 */

/*
 * FIXME: array elements initialized to the same record (instead of allocating a record per
 * element).
 *
 * FIXME: the register allocator sometimes seems to spill a value and reload it right after, so
 * that's a useless spill.
 * TODO: test string equality.
 * FIXME: rdi calle-save register does not seem to be restored (useless spill?).
 * TODO: Clean mov rbx, [rbp + -16] into mov rbx, [rbp - 16].
 * TODO: emit mov, push, mov, push instead of mov, mov, push, push.
 * FIXME: escape analysis (tests/functions.tig) where argument are put in the frame.
 */

#![allow(unknown_lints, clippy::match_like_matches_macro)]
#![deny(clippy::pattern_type_mismatch)]
#![feature(box_patterns)]

mod asm;
mod asm_gen;
mod ast;
mod canon;
mod data_layout;
mod env;
mod error;
mod escape;
mod flow;
mod frame;
mod gen;
mod graph;
mod ir;
mod lexer;
mod liveness;
mod parser;
mod position;
mod reg_alloc;
mod rewriter;
mod semant;
mod symbol;
mod temp;
mod terminal;
mod token;
mod types;

use std::env::args;
use std::fs::{File, read_dir};
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use asm_gen::Gen;
use canon::{basic_blocks, linearize, trace_schedule};
use data_layout::{STRING_DATA_LAYOUT_SIZE, STRING_TYPE};
use env::Env;
use error::Error;
use escape::find_escapes;
use frame::{Fragment, Frame};
use frame::x86_64::X86_64;
use lexer::Lexer;
use parser::Parser;
use reg_alloc::alloc;
use rewriter::Rewriter;
use semant::SemanticAnalyzer;
use symbol::{Strings, Symbols};
use terminal::Terminal;

const END_MARKER: &str = "__tiger_pointer_map_end";
const POINTER_MAP_NAME: &str = "__tiger_pointer_map";

fn main() {
    let strings = Rc::new(Strings::new());
    let mut symbols = Symbols::new(Rc::clone(&strings));
    if let Err(error) = drive(strings, &mut symbols) {
        let terminal = Terminal::new();
        if let Err(error) = error.show(&symbols, &terminal) {
            eprintln!("Error printing errors: {}", error);
        }
    }
}

fn drive(strings: Rc<Strings>, symbols: &mut Symbols<()>) -> Result<(), Error> {
    let mut args = args();
    args.next();
    if let Some(filename) = args.next() {
        let file = BufReader::new(File::open(&filename)?);
        let file_symbol = symbols.symbol(&filename);
        // 1. 词法分析
        let lexer = Lexer::new(file, file_symbol);
        let main_symbol = symbols.symbol("main");
        let self_symbol = symbols.symbol("self");
        let object_symbol = symbols.symbol("Object");
        // 2. 语法分析
        let mut parser = Parser::new(lexer, symbols);
        let ast = parser.parse()?;
        // 3. 实现了一些操作来对表达式（Expr）进行重写。它的目标是让垃圾回收（GC）更方便地收集不再需要的数据。
        let mut rewriter = Rewriter::new(symbols);
        let ast = rewriter.rewrite(ast);
        // 4. 找出所有需要 "逃逸" 的变量
        let escape_env = find_escapes(&ast, Rc::clone(&strings));
        // 5. Env 结构体表示了一个环境，这个环境存储了与编译、类型检查、代码生成等任务相关的信息
        let mut env = Env::<X86_64>::new(&strings, escape_env);
        {
            let semantic_analyzer = SemanticAnalyzer::new(&mut env, Rc::clone(&strings), self_symbol, object_symbol);
            // Fragment 枚举用于表示计算机程序的一部分（例如，函数、字符串或者虚拟表）
            let fragments = semantic_analyzer.analyze(main_symbol, ast)?;

            let mut asm_output_path = PathBuf::from(&filename);
            asm_output_path.set_extension("s");
            let mut file = File::create(&asm_output_path)?;

            writeln!(file, "global main")?;
            writeln!(file, "global {}", POINTER_MAP_NAME)?;
            writeln!(file, "global {}", END_MARKER)?;

            for (function_name, _) in env::external_functions() {
                writeln!(file, "extern {}", function_name)?;
            }
            writeln!(file)?;

            writeln!(file, "section .data")?;
            writeln!(file, "    align 2")?;

            for fragment in &fragments {
                match *fragment {
                    Fragment::Function { .. } => (),
                    Fragment::Str(ref label, ref string) => {
                        // NOTE: creating a useless data layout here so that heap-allocated strings
                        // are accessed the same way as static strings.
                        write!(file, "    {}: ", label)?;
                        writeln!(file, "dq {}", STRING_TYPE)?;
                        for _ in 0..STRING_DATA_LAYOUT_SIZE - 1 {
                            writeln!(file, "dq 0")?;
                        }
                        writeln!(file, "db {}, 0", to_nasm(string))?;
                    },
                    Fragment::VTable { ref class, ref methods } => {
                        writeln!(file, "{}:", class)?;
                        if !methods.is_empty() {
                            let labels = methods.iter()
                                .map(|label| label.to_string())
                                .collect::<Vec<_>>()
                                .join("\n    dq ");
                            writeln!(file, "    dq {}", labels)?;
                        }
                    },
                }
            }

            let mut pointer_map = vec![];

            writeln!(file, "\nsection .text")?;

            for fragment in fragments {
                match fragment {
                    Fragment::Function { body, escaping_vars, frame, temp_map } => {
                        let mut frame = frame.borrow_mut();
                        let body = frame.proc_entry_exit1(body);

                        // 将函数体body转换为一系列线性化的语句，这可能涉及到删除无用的跳转，排序语句等
                        let statements = linearize(body);
                        // 对得到的线性化语句进行基本块分析。基本块是一种在编译器中使用的程序结构，在基本块内部，控制流程是线性的
                        let (basic_blocks, done_label) = basic_blocks(statements);
                        // 对基本块进行跟踪调度，为了改善程序的运行时间
                        let statements = trace_schedule(basic_blocks, done_label);

                        // 使用Gen生成器，将语句转化为目标代码（这里是 X86_64 汇编的表示形式）
                        let mut generator = Gen::<X86_64>::new();
                        for statement in statements {
                            generator.munch_statement(statement);
                        }
                        let instructions = generator.get_result();
                        let instructions = frame.proc_entry_exit2(instructions, escaping_vars);

                        // 调用alloc为使用的临时变量分配物理寄存器或内存空间
                        let (instructions, temp_map) = alloc::<X86_64>(instructions, &mut *frame, temp_map);
                        pointer_map.push(temp_map);

                        let subroutine = frame.proc_entry_exit3(instructions);
                        // 将生成的指令写入文件
                        writeln!(file, "{}", subroutine.prolog)?;
                        for instruction in subroutine.body {
                            let instruction = instruction.to_string::<X86_64>();
                            if !instruction.is_empty() {
                                writeln!(file, "    {}", instruction)?;
                            }
                        }
                        writeln!(file, "    {}", subroutine.epilog)?;
                    },
                    Fragment::Str(_, _) => (),
                    Fragment::VTable { .. } => (),
                }
            }

            writeln!(file)?;

            writeln!(file, "{}:", POINTER_MAP_NAME)?;
            for map in &pointer_map {
                for &(ref label, ref pointer_temps) in map {
                    writeln!(file, "    dq {}", label)?;
                    for temp_label in pointer_temps {
                        writeln!(file, "    dq {}", temp_label.to_label::<X86_64>())?;
                    }
                    writeln!(file, "    dq {}", END_MARKER)?;
                }
            }
            writeln!(file, "    dq {}", END_MARKER)?;
            writeln!(file, "{}:", END_MARKER)?;

            // 这段代码使用了 Rust 的 Command 类来启动一个新的进程执行 nasm 命令。nasm 是一个通用的 x86 汇编器，将汇编源文件转换为机器语言的可执行文件或目标文件。
            let status = Command::new("nasm")
                .args(&["-f", "elf64", asm_output_path.to_str().expect("asm output path")])
                .status();

            match status {
                Ok(return_code) => {
                    if return_code.success() {
                        let mut object_output_path = PathBuf::from(&filename);
                        object_output_path.set_extension("o");
                        let mut executable_output_path = PathBuf::from(&filename);
                        executable_output_path.set_extension("");
                        Command::new("ld")
                            .args(&[
                                "-dynamic-linker", "/lib64/ld-linux-x86-64.so.2", "-o",
                                executable_output_path.to_str().expect("executable output path"),
                                "/usr/lib/Scrt1.o", "/usr/lib/crti.o", &format!("-L{}", get_gcc_lib_dir()?),
                                "-L/usr/lib64/",
                                object_output_path.to_str().expect("object output path"),
                                "target/debug/libruntime.a", "-lpthread", "-ldl", "--no-as-needed", "-lc", "-lgcc", "--as-needed",
                                "-lgcc_s", "--no-as-needed", "/usr/lib/crtn.o"
                            ])
                            .status()
                            .expect("link");
                    }
                },
                Err(error) => eprintln!("Error running nasm: {}", error),
            }
        }
        env.end_scope(); // TODO: move after the semantic analysis?
    }
    Ok(())
}

fn to_nasm(string: &str) -> String {
    let mut result = "'".to_string();
    for char in string.chars() {
        let string =
            match char {
                '\'' | '\n' | '\t' => format!("', {}, '", char as u32),
                _ => char.to_string(),
            };
        result.push_str(&string);
    }
    result.push('\'');
    result
}

fn get_gcc_lib_dir() -> io::Result<String> {
    let directory = "/usr/lib64/gcc/x86_64-pc-linux-gnu/";
    let files = read_dir(directory)?;
    for file in files {
        let file = file?;
        if file.metadata()?.is_dir() {
            return file.file_name().to_str()
                .map(|str| format!("{}{}", directory, str))
                .ok_or_else(|| io::ErrorKind::InvalidData.into());
        }
    }
    Err(io::ErrorKind::NotFound.into())
}

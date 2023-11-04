/*
 * Copyright (c) 2020 Boucher, Antoni <bouanto@zoho.com>
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

use std::io::stderr;
use std::os::raw::c_int;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

const BOLD: &str = "\x1b[1m";
const BLUE: &str = "\x1b[34m";
const END_BOLD: &str = "\x1b[22m";
const RED: &str = "\x1b[31m";
const RESET_COLOR: &str = "\x1b[39;49m";

pub struct Terminal {
    is_a_tty: bool,
}

impl Terminal {
    pub fn new() -> Self {
        Self {
            is_a_tty: stderr_is_a_tty(),
        }
    }

    pub fn bold(&self) -> &str {
        if self.is_a_tty {
            BOLD
        }
        else {
            ""
        }
    }

    pub fn blue(&self) -> &str {
        if self.is_a_tty {
            BLUE
        }
        else {
            ""
        }
    }

    pub fn end_bold(&self) -> &str {
        if self.is_a_tty {
            END_BOLD
        }
        else {
            ""
        }
    }

    pub fn red(&self) -> &str {
        if self.is_a_tty {
            RED
        }
        else {
            ""
        }
    }

    pub fn reset_color(&self) -> &str {
        if self.is_a_tty {
            RESET_COLOR
        }
        else {
            ""
        }
    }
}

#[cfg(unix)]
fn stderr_is_a_tty() -> bool {
    unsafe {
        isatty(stderr().as_raw_fd()) != 0
    }
}

#[cfg(windows)]
fn stderr_is_a_tty() -> bool {
    false
}

extern "C" {
    // `isatty` 是一个在 POSIX 系统（如 Unix、Linux、macOS）中的库函数，用于检查一个文件描述符是否关联到一个终端设备（也称为 TTY）。
    // 在这段 Rust 代码中，`isatty` 函数被声明为一个外部 C 函数。这意味着这个函数实际上是在 C 语言的运行时库中实现的，而 Rust 代码可以调用它。
    // 函数接受一个 `c_int` 类型的参数 `fd`，这是要检查的文件描述符。在 Unix-like 系统中，文件描述符是一个整数，用于代表一个打开的文件或其他类型的 I/O 资源。
    // 函数返回一个 `c_int` 类型的值。如果文件描述符关联到一个终端设备，`isatty` 返回非零值；否则，返回零。
    // 这个函数通常用于确定程序的输出是否应该格式化为适合在终端上显示，或者应该以更适合机器处理的方式（如日志文件或管道到其他程序）进行格式化。
    fn isatty(fd: c_int) -> c_int;
}

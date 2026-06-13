#![no_std]
#![no_main]

mod lexer;
mod parser;
mod runtime;
mod sys;
mod error;
use error::ShellError;

use alloc::{
    string::{String, ToString},
};
use vespertine_abi::ProcessInitPackage;
use vespertine_rt::{print, println, source::read_line};
use vespertine_std::term::get_term_cursor_position;

use crate::{runtime::ShellRuntime, sys::ShellResult};

extern crate alloc;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] shell error: {:?}", e);
    }
}

#[unsafe(no_mangle)]
fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), ShellError> {
    let mut runtime = ShellRuntime::new();

    loop {
        if let Ok((_row, col)) = get_term_cursor_position()
            && col > 0
        {
            // print newline if the last program's output didn't do it
            println!("");
        }

        print!("{} \x1b[35m{} >> \x1b[0m", runtime.context.cwd().to_string(), runtime.context.status());
        let mut buf = [0u8; 128];
        let n = read_line(&mut buf);

        let line = str::from_utf8(&buf[..n])
            .unwrap_or("")
            .trim_end_matches('\n')
            .trim();

        let res = runtime.eval(String::from(line));

        match res {
            Ok(res) => {
                match res {
                    ShellResult::None | ShellResult::Launched(_) => {},
                    other => println!("EVAL: {}", other),
                }
            },
            Err(e) => println!("{}", e),
        }
    }
}


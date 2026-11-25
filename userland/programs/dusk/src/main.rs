#![no_std]
#![no_main]

extern crate alloc;

mod error;
mod lexer;
mod parser;
mod runtime;
mod sys;

use error::ShellError;
use vrt::source::read_line;
use vstd::auth::AuthClient;
use vstd::{Process, prelude::*};
use vstd::term::get_term_cursor_position;

use crate::runtime::ShellRuntime;
use crate::sys::ShellResult;

fn cd_user_home(runtime: &mut ShellRuntime) -> Result<(), Error> {
    let user= Process::info_from_handle(env::self_handle())?.user;
    let account = AuthClient::connect()?.lookup_id(user)?;
    runtime.context.change_dir(Path::new(&account.home).to_path_buf())
}

#[vapp::main]
fn main(_pkg: &ProcessInitPackage) -> Result<(), ShellError> {
    let mut runtime = ShellRuntime::new();
    if let Err(e) = cd_user_home(&mut runtime) {
        println!("dusk: cannot enter home directory: {e}")
    }

    loop {
        if let Ok((_row, col)) = get_term_cursor_position() &&
            col > 0
        {
            // print newline if the last program's output didn't do it
            println!("");
        }

        runtime.draw_prompt();
        let mut buf = [0u8; 128];
        let n = read_line(&mut buf);

        let line = str::from_utf8(&buf[..n]).unwrap_or("").trim_end_matches('\n').trim();

        let res = runtime.eval(String::from(line));

        match res {
            Ok(res) => match res {
                ShellResult::None | ShellResult::Launched(_) => {},
                error @ ShellResult::CommandError(..) => println!("{error}"),
                other => println!("EVAL: {}", other),
            },
            Err(e) => println!("{}", e),
        }
    }
}

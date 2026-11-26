#![no_std]
#![no_main]

extern crate alloc;

mod error;
mod lexer;
mod parser;
mod runtime;
mod sys;

use error::ShellError;
use vespertine_abi::app::termios::*;
use vliner::{LineState, Redraw};
use vrt::source::read_line;
use vstd::auth::AuthClient;
use vstd::{HandleReader, HandleWriter, Process, prelude::*};
use vstd::term::*;

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

    let mut editor = LineState::new(None);
    let input = HandleReader::new(env::source());
    let output = HandleWriter::new(env::sink());

    loop {
        disable_signals().map_err(|_| ShellError::TerminalError)?;
        let prompt = runtime.prompt();
        let Some(line) = read_prompt(&mut editor, &input, &output, &prompt)
            .map_err(|e| {
                println!("dusk: {e}");
                ShellError::TerminalError
            })? 
        else {
            break;
        };
        enable_signals().map_err(|_| ShellError::TerminalError)?;

        let res = runtime.eval(line);

        match res {
            Ok(res) => match res {
                ShellResult::None | ShellResult::Launched(_) => {},
                error @ ShellResult::CommandError(..) => println!("{error}"),
                other => println!("EVAL: {}", other),
            },
            Err(e) => println!("{}", e),
        }
    }

    Ok(())
}

fn read_prompt(
    editor: &mut LineState,
    input: &HandleReader,
    output: &HandleWriter,
    prompt: &str,
) -> Result<Option<String>, Error> {
    let saved = get_terminfo()?;
    let mut mode = saved;
    mode.c_lflag &= !(ICANON | ECHO | ECHONL);
    mode.c_oflag &= !OPOST;
    mode.c_cc[VMIN] = 1;
    mode.c_cc[VTIME] = 0;
    set_terminfo(mode)?;

    let result = read_prompt_in_edit_mode(editor, input, output, prompt);

    // restore the mode even if reading or drawing failed
    let restored = match set_terminfo(saved) {
        Ok(()) => get_terminfo(),
        Err(error) => Err(error),
    };

    let command = result?;
    restored?;
    Ok(command)
}

fn read_prompt_in_edit_mode(
    editor: &mut LineState,
    input: &HandleReader,
    output: &HandleWriter,
    prompt: &str,
) -> Result<Option<String>, Error> {
    // the reply confirms terminal processed the earlier mode change
    let (rows, cols) = get_termsize()?;
    let mut redraw = Redraw::default();

    output.write_all(b"\x1b[N")?;
    redraw.paint(output, prompt, &[], 0, rows, cols)?;

    let command = editor.read_line(input, |text, cursor| {
        redraw.paint(output, prompt, text, cursor, rows, cols)
    })?;

    if command.is_some() {
        output.write_all(b"\r\n")?;
    }

    Ok(command)
}

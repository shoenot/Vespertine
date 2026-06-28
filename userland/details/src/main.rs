#![no_main]
#![no_std]

extern crate alloc;

use vespertine_abi::ProcessInitPackage;
use vespertine_abi::typed::RECORD_PRESENTATION_DETAILS;
use vespertine_cli::args::{
    Command,
    Opt,
};
use vespertine_rt::println;
use vespertine_rt::syscall::sys_close;
use vespertine_std::typed::{
    BufferedRecordStream,
    DateTimeStyle,
    DisplayOptions,
    TypedReader,
    render_record_details,
};
use vespertine_std::{
    Error,
    HandleReader,
    HandleWriter,
    Write,
    env,
};

static DETAILS_OPTIONS: &[Opt] = &[
    Opt::value("datetime", Some('d'), Some("datetime")),
];

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let _pkg = unsafe { &*pkg_ptr };

    if let Err(error) = run() {
        println!("details error: {:?}", error);
    }

    let _ = sys_close(env::sink());
}

fn run() -> Result<(), Error> {
    let args = env::args();
    let matches = Command::new("details")
        .options(DETAILS_OPTIONS)
        .parse(&args[1..])
        .map_err(Error::from)?;

    let requested = matches.positionals();

    let mut display_opts = DisplayOptions::default();

    display_opts.datetime_style = match matches.value("datetime") {
        Some("std") | None => DateTimeStyle::Standard,
        Some("us") => DateTimeStyle::StandardUS,
        Some("iso") => DateTimeStyle::Iso,
        Some("unix") => DateTimeStyle::Unix,
        Some("date") => DateTimeStyle::Date,
        Some("time") => DateTimeStyle::Time,
        Some(_) => return Err(Error::invalid_argument("invalid datetime format".into())),
    };

    let reader = TypedReader::new(HandleReader::new(env::source()));
    let out = HandleWriter::new(env::sink());
    let mut stream = BufferedRecordStream::new();

    stream.read_from(&reader, &out, display_opts)?;

    if let Some(schema) = stream.schema() {
        render_record_details(
            &out,
            schema,
            stream.presentation(RECORD_PRESENTATION_DETAILS),
            requested,
            &stream.rows,
            display_opts,
        )?;
    }

    Ok(())
}

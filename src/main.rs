mod ast;
mod checker;
mod cli;
mod codegen;
mod error;
mod lexer;
mod parser;
mod optimize;

use cli::{build, run, Opts};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法:");
        eprintln!("  lume build <file.lume> [--fastmath] [--native] [--lto] [-O2]");
        eprintln!("  lume run   <file.lume> [--fastmath] [--native] [--lto] [-O2]");
        std::process::exit(1);
    }

    let cmd = &args[1];
    let mut opts = Opts::default();
    opts.fastmath = args.iter().any(|a| a == "--fastmath");
    opts.native = args.iter().any(|a| a == "--native");
    opts.lto = args.iter().any(|a| a == "--lto");
    // 默认 -O3，可以显式降到 -O2
    if args.iter().any(|a| a == "-O2") {
        opts.opt = 2;
    }

    let file = match args.iter().skip(2).find(|a| !a.starts_with("-")) {
        Some(f) => f.clone(),
        None => {
            eprintln!("error: 缺少 .lume 文件");
            std::process::exit(1);
        }
    };

    match cmd.as_str() {
        "build" => build(&file, &opts),
        "run" => run(&file, &opts),
        _ => {
            eprintln!("error: 未知命令 '{}'", cmd);
            eprintln!("用法: lume build|run <file.lume>");
            std::process::exit(1);
        }
    }
}
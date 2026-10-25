mod ast;
mod builtins;
mod checker;
mod cli;
mod codegen;
mod error;
mod lexer;
mod libpkg;
mod ops;
mod parser;
mod optimize;

use cli::{build, list_libs, run, Opts};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法:");
        eprintln!("  lume build <file.lume> [--fastmath] [--native] [--lto] [-O2] [--unchecked]");
        eprintln!("  lume run   <file.lume> [选项...] [-- 程序参数...]");
        eprintln!("  lume libs  <file.lume>   列出库包、依赖关系、本次会加载哪些");
        eprintln!("  --unchecked  关掉 a[i] 的运行期边界检查（默认开，热点内核才用）");
        eprintln!();
        eprintln!("  run 时写在 .lume 之后的参数会原样传给程序（arg/arg_count 取）：");
        eprintln!("    lume run tool.lume in.txt out.txt      # arg(1)=in.txt");
        std::process::exit(1);
    }

    let cmd = &args[1];
    let mut opts = Opts::default();
    opts.fastmath = args.iter().any(|a| a == "--fastmath");
    opts.native = args.iter().any(|a| a == "--native");
    opts.lto = args.iter().any(|a| a == "--lto");
    opts.unchecked = args.iter().any(|a| a == "--unchecked");
    // 默认 -O3，可以显式降到 -O2
    if args.iter().any(|a| a == "-O2") {
        opts.opt = 2;
    }

    // 文件之后的参数透传给程序。`--` 之后的**全部**透传（包括看起来像
    // lume 选项的），方便程序自己需要一个叫 --native 的参数。
    const LUME_FLAGS: &[&str] = &["--fastmath", "--native", "--lto", "-O2", "--unchecked"];
    let mut prog_args: Vec<String> = Vec::new();
    let file = match args.iter().enumerate().skip(2).find(|(_, a)| !a.starts_with('-')) {
        Some((idx, f)) => {
            let rest = &args[idx + 1..];
            let (flag_part, tail) = match rest.iter().position(|a| a == "--") {
                Some(p) => (&rest[..p], &rest[p + 1..]),
                None => (rest, &rest[rest.len()..]),
            };
            prog_args.extend(
                flag_part.iter().filter(|a| !LUME_FLAGS.contains(&a.as_str())).cloned(),
            );
            prog_args.extend(tail.iter().cloned());
            f.clone()
        }
        None => {
            eprintln!("error: 缺少 .lume 文件");
            std::process::exit(1);
        }
    };

    match cmd.as_str() {
        "build" => build(&file, &opts),
        "run" => run(&file, &opts, &prog_args),
        "libs" => list_libs(&file),
        _ => {
            eprintln!("error: 未知命令 '{}'", cmd);
            eprintln!("用法: lume build|run|libs <file.lume>");
            std::process::exit(1);
        }
    }
}
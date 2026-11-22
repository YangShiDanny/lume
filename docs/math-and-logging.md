# Lume math and logging built-ins

Lume exposes scalar math functions to the evaluator and standard library.
Floating-point literals use decimal notation (`0.5`, `3.0`).

## Math API

Unary functions:

`sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `sinh`, `cosh`, `tanh`,
`asinh`, `acosh`, `atanh`, `exp`, `exp2`, `expm1`, `ln`, `log`, `log2`,
`log10`, `log1p`, `sqrt`, `cbrt`, `floor`, `ceil`, `round`, `trunc`, `abs`.

Binary functions:

`pow(x, y)`, `atan2(y, x)`, `hypot(x, y)`, `fmod(x, y)`,
`remainder(x, y)`, and `copysign(x, y)`.

`llvm_sin`, `llvm_cos`, `llvm_exp`, `llvm_log`, and `llvm_sqrt` are aliases
for the compiler-host math implementation. They provide the scalar LLVM/libc
math surface used by Lume; this build does not bundle LLVM libc as a separate
runtime archive.

SLEEF-compatible entry points include `sleef_sin_u10`, `sleef_sin_u35`,
`sleef_cos_u10`, `sleef_cos_u35`, `sleef_tan_u10`, `sleef_exp_u10`,
`sleef_log_u10`, `sleef_sqrt_u05`, `sleef_pow_u10`, and `sleef_pow_u35`.
Without the optional system library, these use the host math implementation.
To use the native SLEEF ABI, build/install SLEEF and compile with:

```sh
LD_LIBRARY_PATH=/path/to/sleef/lib \
SLEEF_LIB_DIR=/path/to/sleef/lib \
  cargo run -p lumec_core --features system-sleef -- \
  run examples/demo_math_logging.lume
```

The native feature links `Sleef_*` scalar functions from `libsleef`.
The fallback does not claim SLEEF's accuracy guarantees; select the native
feature when those guarantees are required.
SLEEF currently exports `Sleef_pow_u10` rather than a separate `pow_u35`
entry point, so both Lume power aliases use the available `u10` symbol when
native linking is enabled.

## Logging API

`trace(message)`, `debug(message)`, `info(message)`, `warn(message)`,
`error(message)`, and `log_at(level, message)` write one level-tagged record to
stderr. `lib/std/log.lume` provides Lume wrappers.

The upstream ferrilog repository currently identifies itself as zerolog-rs,
requires nightly Rust, and pulls additional git workspace dependencies. The
core runtime therefore exposes its level-oriented logging surface without
linking that crate into the stable bootstrap binary.

## Example

```sh
cargo run -p lumec_core -- run examples/demo_math_logging.lume
```

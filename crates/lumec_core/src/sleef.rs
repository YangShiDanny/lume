#[cfg(feature = "system-sleef")]
#[link(name = "sleef")]
unsafe extern "C" {
    #[link_name = "Sleef_sin_u10"]
    pub fn sin_u10(x: f64) -> f64;
    #[link_name = "Sleef_sin_u35"]
    pub fn sin_u35(x: f64) -> f64;
    #[link_name = "Sleef_cos_u10"]
    pub fn cos_u10(x: f64) -> f64;
    #[link_name = "Sleef_cos_u35"]
    pub fn cos_u35(x: f64) -> f64;
    #[link_name = "Sleef_tan_u10"]
    pub fn tan_u10(x: f64) -> f64;
    #[link_name = "Sleef_exp_u10"]
    pub fn exp_u10(x: f64) -> f64;
    #[link_name = "Sleef_log_u10"]
    pub fn log_u10(x: f64) -> f64;
    #[link_name = "Sleef_sqrt_u05"]
    pub fn sqrt_u05(x: f64) -> f64;
    #[link_name = "Sleef_pow_u10"]
    pub fn pow_u10(x: f64, y: f64) -> f64;
}

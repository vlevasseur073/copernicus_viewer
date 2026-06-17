//! Compare two EOPF Zarr products from the command line.
//!
//! Opens a reference and a new product, runs the same comparison logic as the
//! GUI **Tools → Comparison** tool, and prints a text report.
//!
//! ```bash
//! cargo run --example compare_products -- /path/to/reference.zarr /path/to/new.zarr
//! ```
//!
//! Compare the sample product to itself (should pass):
//!
//! ```bash
//! cargo run --example create_sample_zarr
//! cargo run --example compare_products -- \
//!   sample_data/S03OLCEFR_sample.zarr \
//!   sample_data/S03OLCEFR_sample.zarr
//! ```
//!
//! Optional flags (sentineltoolbox-style thresholds):
//!
//! ```bash
//! cargo run --example compare_products -- \
//!   reference.zarr new.zarr \
//!   --relative \
//!   --threshold 0.01 \
//!   --threshold-outliers 0.01 \
//!   --threshold-coverage 0.01
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use copernicus_viewer::comparison::{compare_products_with_options, ComparisonOptions};
use copernicus_viewer::zarr::open_store;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return if args.is_empty() {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }

    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let (reference, new, options, verbose) = parse_args(args)?;

    eprintln!("Opening reference: {}", reference.display());
    let reference_store = open_store(&reference).map_err(|e| e.to_string())?;

    eprintln!("Opening new:       {}", new.display());
    let new_store = open_store(&new).map_err(|e| e.to_string())?;

    eprintln!("Comparing…");
    let result = compare_products_with_options(&reference_store, &new_store, &options);

    println!("{}", result.formatted_summary(verbose));

    if result.success {
        Ok(())
    } else {
        Err("comparison failed".to_string())
    }
}

fn parse_args(args: &[String]) -> Result<(PathBuf, PathBuf, ComparisonOptions, bool), String> {
    let mut positional = Vec::new();
    let mut options = ComparisonOptions::default();
    let mut verbose = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--relative" => options.relative = true,
            "--absolute" => options.absolute = true,
            "--no-structure" => options.structure = false,
            "--no-data" => options.data = false,
            "--no-flags" => options.flags = false,
            "--verbose" | "-v" => verbose = true,
            "--threshold" => {
                i += 1;
                options.threshold = next_f64(args, i, "--threshold")?;
            }
            "--threshold-packed" => {
                i += 1;
                options.threshold_packed = next_f64(args, i, "--threshold-packed")?;
            }
            "--threshold-outliers" => {
                i += 1;
                options.threshold_nb_outliers = next_f64(args, i, "--threshold-outliers")?;
            }
            "--threshold-coverage" => {
                i += 1;
                options.threshold_coverage = next_f64(args, i, "--threshold-coverage")?;
            }
            arg if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            arg => positional.push(PathBuf::from(arg)),
        }
        i += 1;
    }

    if positional.len() < 2 {
        print_help();
        return Err("expected two product paths: reference.zarr new.zarr".to_string());
    }

    let reference = normalize_product_path(&positional[0])?;
    let new = normalize_product_path(&positional[1])?;

    Ok((reference, new, options, verbose))
}

fn normalize_product_path(path: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Err(format!("path not found: {}", path.display()));
    }
    Ok(path.to_path_buf())
}

fn next_f64(args: &[String], index: usize, flag: &str) -> Result<f64, String> {
    args.get(index)
        .ok_or_else(|| format!("{flag} requires a value"))?
        .parse()
        .map_err(|_| format!("invalid number for {flag}"))
}

fn print_help() {
    eprintln!(
        "\
Compare two EOPF Zarr products.

Usage:
  cargo run --example compare_products -- <reference.zarr> <new.zarr> [options]

Options:
  --relative              Force relative error for all variables
  --absolute              Force absolute error for all variables
  --no-structure          Skip structure/metadata checks
  --no-data               Skip variable data comparison
  --no-flags              Skip flag/mask comparison
  --threshold <f>         Data threshold (default: 0.01)
  --threshold-packed <f>  Packed-variable threshold factor (default: 1.5)
  --threshold-outliers <f> Max outlier ratio (default: 0.01)
  --threshold-coverage <f> Max coverage difference ratio (default: 0.01)
  --verbose, -v           List every variable and flag bit
  -h, --help              Show this help
"
    );
}

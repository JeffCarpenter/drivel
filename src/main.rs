use clap::{Parser, Subcommand};
use drivel::{SchemaState, ToJsonSchema};
use serde_json::Value;
use serde_yaml2;
use jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[derive(Subcommand, Debug)]
enum Mode {
    /// Describe the inferred schema for the input data
    Describe {
        /// Output JSON Schema format instead of human-readable description
        #[arg(long)]
        json_schema: bool,
    },
    /// Produce synthetic data adhering to the inferred schema
    Produce {
        #[arg(short, long)]
        /// Produce `n` elements. Default = 1.
        n_repeat: Option<usize>,
    },
}

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    #[command(subcommand)]
    mode: Mode,

    /// Treat input as JSON Schema instead of example data
    #[arg(long, global = true)]
    from_schema: bool,

    /// Infer that some string fields are enums based on the number of unique values seen.
    #[arg(long, global = true)]
    infer_enum: bool,

    /// The maximum ratio of unique values to total values for a field to be considered an enum. Default = 0.1.
    #[arg(long, global = true)]
    enum_max_uniq: Option<f64>,

    /// The minimum sample size of strings before enum inference will be attempted. Default = 1.
    #[arg(long, global = true)]
    enum_min_n: Option<usize>,
}

impl From<&Args> for Option<drivel::EnumInference> {
    fn from(value: &Args) -> Self {
        if value.infer_enum {
            let max_unique_ratio = value.enum_max_uniq.unwrap_or(0.1);
            let min_sample_size = value.enum_min_n.unwrap_or(1);
            Some(drivel::EnumInference {
                max_unique_ratio,
                min_sample_size,
            })
        } else {
            None
        }
    }
}

fn parse_json_or_yaml(s: &str) -> Result<Value, String> {
    match serde_json::from_str(s) {
        Ok(v) => Ok(v),
        Err(json_err) => match serde_yaml2::from_str::<serde_yaml2::wrapper::YamlNodeWrapper>(s) {
            Ok(node) => serde_json::to_value(&node).map_err(|e| e.to_string()),
            Err(yaml_err) => Err(format!(
                "JSON error: {}. YAML error: {}",
                json_err, yaml_err
            )),
        },
    }
}

fn main() {
    let args = Args::parse();
    let input = match std::io::read_to_string(std::io::stdin()) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Unable to read from stdin. Error: {}", err);
            std::process::exit(1)
        }
    };

    let schema = if args.from_schema {
        // Parse input as JSON Schema (JSON or YAML)
        let json = match parse_json_or_yaml(&input) {
            Ok(json) => json,
            Err(err) => {
                eprintln!("Error parsing input as JSON or YAML Schema: {}", err);
                std::process::exit(1);
            }
        };

        match drivel::parse_json_schema(&json) {
            Ok(schema) => schema,
            Err(err) => {
                eprintln!("Error parsing JSON Schema: {}", err);
                std::process::exit(1);
            }
        }
    } else {
        // Existing inference workflow
        let opts = drivel::InferenceOptions {
            enum_inference: (&args).into(),
        };

        if let Ok(json) = parse_json_or_yaml(&input) {
            drivel::infer_schema(json, &opts)
        } else {
            // unable to parse input as single document; try line-based format
            let values = input
                .lines()
                .map(|line| match parse_json_or_yaml(line) {
                    Ok(v) => v,
                    Err(err) => {
                        eprintln!(
                            "Error parsing input; are you sure it is valid JSON or YAML? Error: {}",
                            err
                        );
                        std::process::exit(1);
                    }
                })
                .collect();
            drivel::infer_schema_from_iter(values, &opts)
        }
    };

    match &args.mode {
        Mode::Produce { n_repeat } => {
            let n_repeat = n_repeat.unwrap_or(1);
            let schema = match schema {
                SchemaState::Array { .. } => schema,
                _ => {
                    // if the user wants to repeat the data more than once and we aren't dealing
                    // with an array at the root, then we wrap the state in an array before we
                    // produce our values
                    if n_repeat > 1 {
                        SchemaState::Array {
                            min_length: 1,
                            max_length: 1,
                            schema: Box::new(schema),
                        }
                    } else {
                        schema
                    }
                }
            };

            let result = drivel::produce(&schema, n_repeat);
            let stdout = std::io::stdout();
            serde_json::to_writer_pretty(stdout, &result).unwrap();
        }
        Mode::Describe { json_schema } => {
            if *json_schema {
                let json_schema = schema.to_json_schema_document();
                let stdout = std::io::stdout();
                serde_json::to_writer_pretty(stdout, &json_schema).unwrap();
            } else {
                println!("{}", schema.to_string_pretty());
            }
        }
    }
}

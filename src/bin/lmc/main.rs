mod arguments;
mod current;
mod pipeline;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

enum SubcommandArguments {
    Resistance(pipeline::PipelineArguments),
    Current(current::CurrentArguments),
    Full(pipeline::PipelineArguments),
}

fn main() {
    let arguments = std::env::args_os().collect();
    let subcommand_arguments = arguments::parse(arguments).unwrap_or_else(|error| error.exit());
    let result = match subcommand_arguments {
        SubcommandArguments::Resistance(arguments) | SubcommandArguments::Full(arguments) => {
            pipeline::execute(arguments)
        }
        SubcommandArguments::Current(arguments) => current::execute(arguments),
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests;

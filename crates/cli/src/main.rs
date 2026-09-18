use clap::Parser;

#[derive(Parser)]
#[command(name = "anthrex", version, about = "A terminal multiplexer for coding agents")]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    println!("anthrex {} (proto {})", env!("CARGO_PKG_VERSION"), proto::PROTO_VERSION);
}

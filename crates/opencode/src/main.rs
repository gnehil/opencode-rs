use opencode::cli;

#[tokio::main]
async fn main() {
    cli::run_async().await;
}

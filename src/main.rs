use blossom::config::Config;
use blossom::state::AppState;
use blossom::{public, sweep, takedown, upload};
use clap::Parser;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::parse();
    let write_listener = TcpListener::bind(config.write_addr).await?;
    let public_listener = TcpListener::bind(config.public_addr).await?;
    println!("write port listening on {}", write_listener.local_addr()?);
    println!("public port listening on {}", public_listener.local_addr()?);

    let state = AppState::open(config)?;
    takedown::apply(&state)?;
    let sweeper = sweep::run(
        state.clone(),
        std::time::Duration::from_secs(state.config.sweep_interval_secs),
    );
    let write = axum::serve(write_listener, upload::router(state.clone()));
    let public = axum::serve(public_listener, public::router(state));
    tokio::select! {
        result = write => result?,
        result = public => result?,
        () = sweeper => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    Ok(())
}

//! `anthrex ls`: the window list, as a table or as pretty JSON.

use std::path::Path;

use crate::client;

pub async fn ls(socket: &Path, json: bool) -> anyhow::Result<()> {
    let c = client::CliClient::connect(socket).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&c.windows)?);
    } else {
        print!("{}", client::format_table(&c.windows));
    }
    Ok(())
}

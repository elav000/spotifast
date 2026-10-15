//! Manual, memory-only Jellyfin connection probe. No Spotify grant required.
use std::time::Duration;

use spotifast::jellyfin::{Client, Error};
use spotifast::settings::ProxyConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = std::env::args().nth(1).ok_or_else(|| {
        anyhow::anyhow!("Usage: cargo run --example jellyfin_browse -- <server-url>")
    })?;
    // A stable diagnostic identity, independent of the future app account IDs.
    let client = Client::new(
        &server,
        "spotifast-jellyfin-browse-probe",
        &ProxyConfig::System,
    )?;
    let info = client.server_info().await?;
    println!(
        "Connected to {}",
        info.server_name.as_deref().unwrap_or("Jellyfin")
    );
    let challenge = client.initiate().await?;
    println!(
        "Approve Quick Connect code {} in your signed-in Jellyfin client.",
        challenge.code
    );
    println!("This probe keeps the token only in memory. Press Ctrl-C to cancel.");
    let account = tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            match client.finish(&challenge).await {
                Ok(account) => break Ok(account),
                Err(Error::Pending) => tokio::time::sleep(Duration::from_secs(5)).await,
                Err(error) => break Err(error),
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("Quick Connect timed out. Run the probe again to retry."))??;
    let libraries = client
        .music_libraries(&account.user_id, &account.token)
        .await?;
    if libraries.is_empty() {
        println!("This account has no accessible music libraries.");
    }
    for library in libraries {
        println!("\nLibrary: {}", library.name);
        // Deliberately bounded diagnostic output, not a full-catalog sync.
        let albums = client
            .albums(&account.user_id, &library.id, 0, &account.token)
            .await?;
        println!("{} albums; showing up to ten", albums.total_record_count);
        for album in albums.items.iter().take(10) {
            println!("  {}", album.name.as_deref().unwrap_or("Untitled album"));
        }
        if let Some(album) = albums.items.first() {
            let tracks = client
                .album_tracks(&account.user_id, &album.id, 0, &account.token)
                .await?;
            println!(
                "First album: {} tracks; showing up to ten",
                tracks.total_record_count
            );
            for track in tracks.items.iter().take(10) {
                println!("    {}", track.name.as_deref().unwrap_or("Untitled track"));
            }
        }
    }
    Ok(())
}

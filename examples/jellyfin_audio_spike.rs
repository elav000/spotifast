//! Standalone, single-track decoder/output experiment. The app remains unchanged.
fn main() -> anyhow::Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let audible = arguments.iter().any(|argument| argument == "--audible");
    let container = arguments
        .iter()
        .find_map(|argument| argument.strip_prefix("--format="));
    if container.is_some_and(|kind| !["flac", "mp3", "wav"].contains(&kind)) {
        anyhow::bail!("Supported format filters: --format=flac, --format=mp3, --format=wav");
    }
    let library = arguments
        .iter()
        .find(|argument| !argument.starts_with("--"));
    println!("Testing one track from the selected library (first album, or requested format).");
    if audible {
        println!(
            "Playing about six seconds at low volume, including a seek to 90%. Close other local players first."
        );
    } else {
        println!("Silent decode and EQ/visualizer test. Add --audible to use the audio device.");
    }
    let report = spotifast::backend::jellyfin::probe_saved_playback(
        &spotifast::paths::AppDirs::discover().dirs,
        &spotifast::settings::ProxyConfig::System,
        library.map(String::as_str),
        audible,
        container,
    )
    .map_err(anyhow::Error::msg)?;
    println!("Track: {}", report.title);
    println!(
        "Catalog format: {:?}, codec: {:?}; decoded: {} Hz, {} channels",
        report.container, report.codec, report.sample_rate, report.channels
    );
    println!(
        "First PCM: {} ms, after {} / {} encoded bytes",
        report.first_pcm_ms, report.bytes_at_first_pcm, report.file_bytes
    );
    println!(
        "Seek to 90%: {} ms (approximate: {}); duration: {} ms",
        report.seek_ms, report.approximate_seek, report.duration_ms
    );
    println!(
        "Transfer: {} requests, {} bytes; largest range: {} bytes",
        report.transfer.requests, report.transfer.bytes, report.transfer.peak_range_bytes
    );
    println!(
        "Visualizer tap received post-EQ samples: {}",
        report.visualizer_received_samples
    );
    Ok(())
}

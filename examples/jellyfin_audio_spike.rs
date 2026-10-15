//! Standalone, single-track decoder/output experiment. The app remains unchanged.
fn main() -> anyhow::Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let audible = arguments.iter().any(|argument| argument == "--audible");
    let library = arguments
        .iter()
        .find(|argument| argument.as_str() != "--audible");
    println!("Testing the first track of the first album in the selected library.");
    if audible {
        println!(
            "Playing about six seconds at low volume, including a seek to 90%. Close other local players first."
        );
    } else {
        println!("Silent decode and EQ/visualizer test. Add --audible to use the audio device.");
    }
    let report = spotifast::backend::jellyfin::probe_saved_playback(
        &spotifast::paths::AppDirs::discover(),
        &spotifast::settings::ProxyConfig::System,
        library.map(String::as_str),
        audible,
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
        "Seek to 90%: {} ms; duration: {} ms",
        report.seek_ms, report.duration_ms
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

# Synthetic audio fixtures

These files are generated test signals, not recordings of copyrighted songs.
They require no live server, credentials, audio device or encoder at test time.

Generation commands (FFmpeg):

```sh
ffmpeg -f lavfi -i 'anoisesrc=duration=8:sample_rate=44100:seed=42:amplitude=0.2' -ac 2 -c:a flac jellyfin-noise.flac
ffmpeg -f lavfi -i 'sine=frequency=440:sample_rate=48000:duration=1' -ac 1 -c:a pcm_s16le jellyfin-tone.wav
ffmpeg -f lavfi -i 'sine=frequency=440:sample_rate=44100:duration=1' -ac 2 -c:a libmp3lame -b:a 128k jellyfin-tone.mp3
```

The FLAC deliberately exceeds multiple 256 KiB ranges, so first-sample and seek
tests can prove the reader does not download the whole file first. The WAV
exercises mono-to-stereo conversion and 48 kHz to 44.1 kHz resampling, including
the filter tail. The MP3 exercises byte probing and seek preroll.

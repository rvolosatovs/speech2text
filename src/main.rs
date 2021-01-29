use audrey::Reader;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use dasp::sample::Sample;
use dasp::Signal;
use deepspeech::Model;
use fvad::Fvad;
use std::env;
use std::{convert::TryInto, fs::File, path::PathBuf, str::FromStr, sync::mpsc, time::SystemTime};
use structopt::StructOpt;

enum FvadSampleLength {
    Length10ms = 10,
    Length20ms = 20,
    Length30ms = 30,
}

impl FromStr for FvadSampleLength {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, <Self as FromStr>::Err> {
        match s {
            "10" => Ok(Self::Length10ms),
            "10ms" => Ok(Self::Length10ms),
            "10 ms" => Ok(Self::Length10ms),

            "20" => Ok(Self::Length20ms),
            "20ms" => Ok(Self::Length20ms),
            "20 ms" => Ok(Self::Length20ms),

            "30" => Ok(Self::Length30ms),
            "30ms" => Ok(Self::Length30ms),
            "30 ms" => Ok(Self::Length30ms),

            _ => Err(format!(
                "failed to parse `{}` into Fvad sample rate of 10, 20 or 30 ms",
                s
            )),
        }
    }
}

enum FvadMode {
    Quality = 0,
    LowBitrate = 1,
    Aggressive = 2,
    VeryAggressive = 3,
}

impl Into<fvad::Mode> for FvadMode {
    fn into(self) -> fvad::Mode {
        match self {
            Self::Quality => fvad::Mode::Quality,
            Self::LowBitrate => fvad::Mode::LowBitrate,
            Self::Aggressive => fvad::Mode::Aggressive,
            Self::VeryAggressive => fvad::Mode::VeryAggressive,
        }
    }
}

impl FromStr for FvadMode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, <Self as FromStr>::Err> {
        match s {
            "0" => Ok(Self::Quality),
            "quality" => Ok(Self::Quality),

            "1" => Ok(Self::LowBitrate),
            "low bitrate" => Ok(Self::LowBitrate),
            "low-bitrate" => Ok(Self::LowBitrate),
            "low_bitrate" => Ok(Self::LowBitrate),
            "lowbitrate" => Ok(Self::LowBitrate),

            "2" => Ok(Self::Aggressive),
            "aggressive"  => Ok(Self::Aggressive),

            "3" => Ok(Self::VeryAggressive),
            "very aggressive" => Ok(Self::VeryAggressive),
            "very-aggressive" => Ok(Self::VeryAggressive),
            "very_aggressive" => Ok(Self::VeryAggressive),
            "veryaggressive" => Ok(Self::VeryAggressive),

            _ => Err(format!(
                "failed to parse `{}` into Fvad mode of 0 (quality), 1 (low bitrate), 2 (aggresive) or 3 (very aggresive)",
                s
            )),
        }
    }
}

#[derive(StructOpt)]
#[structopt(name = "speech2text", about = "Record voice and print text to stdout.")]
struct Opt {
    /// Enable debugging
    #[structopt(short, long)]
    debug: bool,

    /// Path to model
    #[structopt(short, long, parse(from_os_str))]
    model: PathBuf,

    /// Path to recording file
    #[structopt(short, long, parse(from_os_str))]
    file: Option<PathBuf>,

    /// Fvad sample length in milliseconds: only values of 10, 20 or 30 ms are supported.
    #[structopt(long, default_value = "10ms")]
    fvad_sample_length: FvadSampleLength,

    /// Fvad mode
    #[structopt(long)]
    fvad_mode: Option<FvadMode>,
}

fn main() {
    let opt = Opt::from_args();

    let mut model = Model::load_from_files(&opt.model).expect("Failed to load Deepspeech model");

    let sample_rate = model.get_sample_rate() as u32;
    let channels: u16 = 1;
    let bits_per_sample: u16;

    // input_stream is necessary to prevent the value from being dropped at the end of conditional
    // scope.
    #[allow(unused_variables)]
    let input_stream: _;
    let (tx, rx) = mpsc::channel();
    if let Some(path) = opt.file {
        let mut reader = Reader::new(File::open(path).expect("Failed to open input file"))
            .expect("Failed to read input file");

        let desc = reader.description();
        assert_eq!(desc.channel_count(), channels as u32);
        assert_eq!(
            desc.sample_rate(),
            sample_rate,
            "Sample rate of input file must equal sample rate expected by the model"
        );
        bits_per_sample = 16;

        for s in reader.samples() {
            tx.send(s.expect("Failed to read sample from input file"))
                .expect("Failed to send sample from input stream");
        }
        drop(tx)
    } else {
        let host = cpal::default_host();
        let input_device = host
            .default_input_device()
            .expect("Failed to find default input device");

        let input_stream_conf = input_device
            .supported_input_configs()
            .expect("Failed to get supported device input configurations")
            .find(|x| x.channels() == channels && x.sample_format() == cpal::SampleFormat::I16)
            .expect(
                "Failed to find a single-channel input stream configuration with i16 sample format",
            )
            .with_sample_rate(cpal::SampleRate(sample_rate));
        bits_per_sample = (input_stream_conf.sample_format().sample_size() * 8) as _;

        input_stream = input_device
            .build_input_stream(
                &input_stream_conf.config(),
                move |data: &[i16], _| {
                    for sample in data {
                        tx.send(sample.to_sample::<i16>())
                            .expect("Failed to send sample from input stream")
                    }
                },
                move |err| eprintln!("Failed to capture frame on input stream: {}", err),
            )
            .expect("Failed to build input stream");
        input_stream.play().expect("Failed to play input stream");
    }

    let vad_sample_rate = match sample_rate / 8000 {
        1 => 8000,
        2 | 3 => 16000,
        4 | 5 => 32000,
        6 => 48000,
        _ => todo!("handling of sample rate {}", sample_rate),
    };
    let mut vad = Fvad::new().expect("Failed to create Fvad").set_sample_rate(
        vad_sample_rate
            .try_into()
            .expect("Failed to set Fvad sample rate"),
    );
    if let Some(mode) = opt.fvad_mode {
        vad = vad.set_mode(mode.into());
    }

    let frame_sample_count = (opt.fvad_sample_length as u32 * (vad_sample_rate / 1000)) as usize;
    let mut signal = dasp::signal::from_iter(rx.iter()).buffered(dasp::ring_buffer::Bounded::from(
        vec![0; frame_sample_count],
    ));

    let mut buffer = Vec::new();
    let mut silence_frames = 0;
    let mut speech_frames = 0;
    while !signal.is_exhausted() {
        let mut frame = signal.next_frames().collect::<Vec<i16>>();

        let is_voice = vad
            .is_voice_frame(&frame)
            .expect("Invalid frame received from input stream");
        buffer.append(&mut frame);

        if is_voice {
            speech_frames += 1;
            silence_frames = 0;
            continue;
        }
        silence_frames += 1;
        // TODO: Make amount of silence "padding" configurable.
        // Let the user specify duration and compute frame count.
        const SILENCE_PADDING: usize = 20;
        if speech_frames == 0 {
            if silence_frames > SILENCE_PADDING {
                buffer = buffer[buffer.len() - frame_sample_count * SILENCE_PADDING..].to_vec();
                silence_frames = SILENCE_PADDING;
            }
            continue;
        }
        if silence_frames < SILENCE_PADDING {
            continue;
        }

        if opt.debug {
            let mut writer = hound::WavWriter::create(
                PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join(format!(
                    "recordings/recording{}.wav",
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .expect("SystemTime before UNIX EPOCH!")
                        .as_nanos()
                )),
                hound::WavSpec {
                    channels,
                    sample_rate,
                    bits_per_sample,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .expect("Failed to create WAV writer");
            for &sample in &buffer {
                writer.write_sample(sample).expect("Failed to write to WAV");
            }
        }
        println!(
            "{}",
            model
                .speech_to_text(&buffer)
                .expect("Failed to process frame"),
        );
        buffer.clear();
        silence_frames = 0;
        speech_frames = 0;
    }
}

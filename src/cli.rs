use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "rs-torrent", about = "BitTorrent CLI downloader")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Decode {
        bencoded_value: String,
    },
    Info {
        torrent_file: String,
    },
    Peers {
        torrent_file: String,
    },
    Handshake {
        torrent_file: String,
        peer: String,
    },
    #[command(name = "download_piece")]
    DownloadPiece(DownloadPieceArgs),
    Download(DownloadArgs),
    #[command(name = "magnet_parse")]
    MagnetParse {
        magnet_link: String,
    },
    #[command(name = "magnet_handshake")]
    MagnetHandshake {
        magnet_link: String,
    },
    #[command(name = "magnet_info")]
    MagnetInfo {
        magnet_link: String,
    },
    #[command(name = "magnet_download_piece")]
    MagnetDownloadPiece(DownloadPieceArgs),
    #[command(name = "magnet_download")]
    MagnetDownload(DownloadArgs),
}

#[derive(Debug, Args)]
pub struct DownloadPieceArgs {
    #[arg(short = 'o', long = "output")]
    pub output_file: String,
    pub source: String,
    pub piece_index: usize,
}

#[derive(Debug, Args)]
pub struct DownloadArgs {
    #[arg(short = 'o', long = "output")]
    pub output_file: String,
    pub source: String,
}

pub fn parse_cli() -> Cli {
    Cli::parse()
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::{CommandFactory, Parser};

    #[test]
    fn parses_download_piece_with_output_flag() {
        let cli = Cli::try_parse_from([
            "app",
            "download_piece",
            "-o",
            "piece.bin",
            "sample.torrent",
            "3",
        ])
        .expect("download-piece should parse");

        match cli.command {
            Command::DownloadPiece(args) => {
                assert_eq!(args.output_file, "piece.bin");
                assert_eq!(args.source, "sample.torrent");
                assert_eq!(args.piece_index, 3);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn help_lists_magnet_commands() {
        let mut command = Cli::command();
        let help = command.render_help().to_string();

        assert!(help.contains("magnet_parse"));
        assert!(help.contains("magnet_handshake"));
        assert!(help.contains("magnet_download"));
    }
}

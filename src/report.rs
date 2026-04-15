#[derive(Debug, PartialEq, Eq)]
pub enum ProgressEvent<'a> {
    ResumeLoaded {
        completed_pieces: usize,
        total_pieces: usize,
    },
    BitfieldProbeFailed {
        peer: &'a str,
        error: String,
    },
    TrackerSelected {
        tracker: &'a str,
    },
    TrackerWarning {
        message: &'a str,
    },
    PieceStored {
        piece_index: usize,
        completed_pieces: usize,
        total_pieces: usize,
    },
    PieceDownloadFailed {
        piece_index: usize,
        peer: &'a str,
        error: String,
    },
    PieceVerificationFailed {
        piece_index: usize,
        peer: &'a str,
    },
    DownloadFinalized {
        output: &'a str,
    },
    DownloadInterrupted {
        output: &'a str,
    },
    PieceWritten {
        piece_index: usize,
        output: &'a str,
    },
}

pub fn render_event(event: &ProgressEvent<'_>) -> String {
    match event {
        ProgressEvent::ResumeLoaded {
            completed_pieces,
            total_pieces,
        } => format!(
            "[progress] resume state loaded: {completed_pieces}/{total_pieces} pieces already complete"
        ),
        ProgressEvent::BitfieldProbeFailed { peer, error } => {
            format!("[warn] failed to probe peer bitfield from {peer}: {error}")
        }
        ProgressEvent::TrackerSelected { tracker } => {
            format!("[progress] selected tracker: {tracker}")
        }
        ProgressEvent::TrackerWarning { message } => {
            format!("[warn] tracker fallback detail: {message}")
        }
        ProgressEvent::PieceStored {
            piece_index,
            completed_pieces,
            total_pieces,
        } => format!(
            "[progress] stored piece {piece_index} ({completed_pieces}/{total_pieces} complete)"
        ),
        ProgressEvent::PieceDownloadFailed {
            piece_index,
            peer,
            error,
        } => format!("[warn] failed to download piece {piece_index} from {peer}: {error}"),
        ProgressEvent::PieceVerificationFailed { piece_index, peer } => format!(
            "[warn] piece {piece_index} failed verification after download from {peer}"
        ),
        ProgressEvent::DownloadFinalized { output } => {
            format!("[progress] finalized download to {output}")
        }
        ProgressEvent::DownloadInterrupted { output } => format!(
            "[warn] download interrupted; partial state preserved for resume at {output}"
        ),
        ProgressEvent::PieceWritten { piece_index, output } => {
            format!("[progress] wrote piece {piece_index} to {output}")
        }
    }
}

pub fn emit_stdout(event: &ProgressEvent<'_>) {
    println!("{}", render_event(event));
}

pub fn emit_stderr(event: &ProgressEvent<'_>) {
    eprintln!("{}", render_event(event));
}

#[cfg(test)]
mod tests {
    use super::{render_event, ProgressEvent};

    #[test]
    fn renders_resume_progress() {
        let rendered = render_event(&ProgressEvent::ResumeLoaded {
            completed_pieces: 2,
            total_pieces: 5,
        });

        assert_eq!(
            rendered,
            "[progress] resume state loaded: 2/5 pieces already complete"
        );
    }

    #[test]
    fn renders_warning_messages() {
        let rendered = render_event(&ProgressEvent::PieceDownloadFailed {
            piece_index: 3,
            peer: "127.0.0.1:6881",
            error: "timeout".into(),
        });

        assert_eq!(
            rendered,
            "[warn] failed to download piece 3 from 127.0.0.1:6881: timeout"
        );
    }

    #[test]
    fn renders_tracker_selection() {
        let rendered = render_event(&ProgressEvent::TrackerSelected {
            tracker: "udp://tracker.test:6969/announce",
        });

        assert_eq!(
            rendered,
            "[progress] selected tracker: udp://tracker.test:6969/announce"
        );
    }

    #[test]
    fn renders_tracker_warning() {
        let rendered = render_event(&ProgressEvent::TrackerWarning {
            message: "udp://a failed | http://b timed out",
        });

        assert_eq!(
            rendered,
            "[warn] tracker fallback detail: udp://a failed | http://b timed out"
        );
    }

    #[test]
    fn renders_interruption_message() {
        let rendered = render_event(&ProgressEvent::DownloadInterrupted {
            output: "output.bin",
        });

        assert_eq!(
            rendered,
            "[warn] download interrupted; partial state preserved for resume at output.bin"
        );
    }
}

// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use rekordcrate::util::FileType;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedFormat {
    Mp3,
    Wav,
    Aiff,
    M4a,
    Flac,
}

impl TryFrom<&str> for SupportedFormat {
    type Error = String;

    fn try_from(ext: &str) -> Result<Self, Self::Error> {
        match ext.to_ascii_lowercase().as_str() {
            "mp3" => Ok(Self::Mp3),
            "wav" => Ok(Self::Wav),
            "aiff" | "aif" => Ok(Self::Aiff),
            "aac" | "m4a" => Ok(Self::M4a),
            "flac" => Ok(Self::Flac),
            _ => Err(format!("unknown format '{ext}'")),
        }
    }
}

impl From<SupportedFormat> for &'static str {
    fn from(fmt: SupportedFormat) -> Self {
        match fmt {
            SupportedFormat::Mp3 => "mp3",
            SupportedFormat::Wav => "wav",
            SupportedFormat::Aiff => "aiff",
            SupportedFormat::M4a => "m4a",
            SupportedFormat::Flac => "flac",
        }
    }
}

impl From<SupportedFormat> for FileType {
    fn from(fmt: SupportedFormat) -> Self {
        match fmt {
            SupportedFormat::Mp3 => FileType::Mp3,
            SupportedFormat::Wav => FileType::Wav,
            SupportedFormat::Aiff => FileType::Aiff,
            SupportedFormat::M4a => FileType::M4a,
            SupportedFormat::Flac => FileType::Flac,
        }
    }
}

impl fmt::Display for SupportedFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str((*self).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_round_trips() {
        for fmt in [
            SupportedFormat::Mp3,
            SupportedFormat::Wav,
            SupportedFormat::Aiff,
            SupportedFormat::M4a,
            SupportedFormat::Flac,
        ] {
            let ext: &str = fmt.into();
            assert_eq!(SupportedFormat::try_from(ext), Ok(fmt));
        }
    }

    #[test]
    fn extension_aliases_and_case() {
        assert_eq!(SupportedFormat::try_from("AIF"), Ok(SupportedFormat::Aiff));
        assert_eq!(SupportedFormat::try_from("aac"), Ok(SupportedFormat::M4a));
        assert!(SupportedFormat::try_from("ogg").is_err());
    }
}

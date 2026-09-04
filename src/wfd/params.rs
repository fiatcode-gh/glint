//! The four Wi-Fi Display RTSP parameter payloads, as types.
//!
//! Each type parses and formats the parameter's **value** — the text after
//! `wfd_video_formats: ` and friends — not the whole header line.
//!
//! Canonical form: lowercase hexadecimal, fixed field widths, one space
//! between fields, `", "` between list entries, list order preserved.
//! `format(parse(s)) == s` holds for canonical input; for non-canonical input
//! (uppercase hex, for instance) parsing normalises, and
//! `parse(format(parse(s))) == parse(s)` holds instead.

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParamError {
    #[error("expected {expected} fields in {context}, found {found}")]
    FieldCount {
        context: &'static str,
        expected: usize,
        found: usize,
    },
    #[error("field {field:?} in {context} is not valid hexadecimal")]
    NotHex {
        context: &'static str,
        field: String,
    },
    #[error("field {field:?} in {context} is not a valid number")]
    NotNumeric {
        context: &'static str,
        field: String,
    },
    #[error("{context} has an unexpected shape: {value:?}")]
    Shape {
        context: &'static str,
        value: String,
    },
}

/// Parse and format a WFD parameter value.
pub trait WfdParam: Sized {
    fn parse(value: &str) -> Result<Self, ParamError>;
    fn format(&self) -> String;
}

fn hex<T: TryFrom<u64>>(context: &'static str, field: &str) -> Result<T, ParamError> {
    if field.is_empty() || !field.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ParamError::NotHex {
            context,
            field: field.to_string(),
        });
    }
    let raw = u64::from_str_radix(field, 16).map_err(|_| ParamError::NotHex {
        context,
        field: field.to_string(),
    })?;
    T::try_from(raw).map_err(|_| ParamError::NotHex {
        context,
        field: field.to_string(),
    })
}

/// `none` (case-insensitive) or a hex value.
fn hex_or_none(context: &'static str, field: &str) -> Result<Option<u16>, ParamError> {
    if field.eq_ignore_ascii_case("none") {
        Ok(None)
    } else {
        hex::<u16>(context, field).map(Some)
    }
}

// ---------------------------------------------------------------- video

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264Codec {
    pub profile: u8,
    pub level: u8,
    pub cea: u32,
    pub vesa: u32,
    pub hh: u32,
    pub latency: u8,
    pub min_slice_size: u16,
    pub slice_enc_params: u16,
    pub frame_rate_control: u8,
    pub max_hres: Option<u16>,
    pub max_vres: Option<u16>,
}

const CODEC_FIELDS: usize = 11;

impl H264Codec {
    fn parse_fields(fields: &[&str]) -> Result<Self, ParamError> {
        const CTX: &str = "wfd_video_formats codec entry";
        if fields.len() != CODEC_FIELDS {
            return Err(ParamError::FieldCount {
                context: CTX,
                expected: CODEC_FIELDS,
                found: fields.len(),
            });
        }
        Ok(H264Codec {
            profile: hex(CTX, fields[0])?,
            level: hex(CTX, fields[1])?,
            cea: hex(CTX, fields[2])?,
            vesa: hex(CTX, fields[3])?,
            hh: hex(CTX, fields[4])?,
            latency: hex(CTX, fields[5])?,
            min_slice_size: hex(CTX, fields[6])?,
            slice_enc_params: hex(CTX, fields[7])?,
            frame_rate_control: hex(CTX, fields[8])?,
            max_hres: hex_or_none(CTX, fields[9])?,
            max_vres: hex_or_none(CTX, fields[10])?,
        })
    }

    fn format_into(&self, out: &mut String) {
        let res = |v: Option<u16>| match v {
            Some(v) => format!("{v:04x}"),
            None => "none".to_string(),
        };
        let _ = write!(
            out,
            "{:02x} {:02x} {:08x} {:08x} {:08x} {:02x} {:04x} {:04x} {:02x} {} {}",
            self.profile,
            self.level,
            self.cea,
            self.vesa,
            self.hh,
            self.latency,
            self.min_slice_size,
            self.slice_enc_params,
            self.frame_rate_control,
            res(self.max_hres),
            res(self.max_vres),
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFormats {
    pub native: u8,
    pub preferred_display_mode: u8,
    pub codecs: Vec<H264Codec>,
}

impl WfdParam for VideoFormats {
    fn parse(value: &str) -> Result<Self, ParamError> {
        const CTX: &str = "wfd_video_formats";
        // `str::split` always yields at least one item, even for the empty
        // string, so the first entry has no missing case to handle.
        let mut entries = value.split(',').map(str::trim);
        let first = entries.next().expect("split always yields one entry");

        let first_fields: Vec<&str> = first.split_whitespace().collect();
        if first_fields.len() != CODEC_FIELDS + 2 {
            return Err(ParamError::FieldCount {
                context: CTX,
                expected: CODEC_FIELDS + 2,
                found: first_fields.len(),
            });
        }
        let native = hex(CTX, first_fields[0])?;
        let preferred_display_mode = hex(CTX, first_fields[1])?;

        let mut codecs = vec![H264Codec::parse_fields(&first_fields[2..])?];
        for entry in entries {
            let fields: Vec<&str> = entry.split_whitespace().collect();
            codecs.push(H264Codec::parse_fields(&fields)?);
        }

        Ok(VideoFormats {
            native,
            preferred_display_mode,
            codecs,
        })
    }

    fn format(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "{:02x} {:02x} ",
            self.native, self.preferred_display_mode
        );
        for (index, codec) in self.codecs.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            codec.format_into(&mut out);
        }
        out
    }
}

// ---------------------------------------------------------------- audio

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioCodec {
    /// `LPCM`, `AAC`, `AC3` — or whatever a newer sink names. Kept verbatim so
    /// an unknown name survives the round trip.
    pub format: String,
    pub modes: u32,
    pub latency: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioCodecs(pub Vec<AudioCodec>);

impl WfdParam for AudioCodecs {
    fn parse(value: &str) -> Result<Self, ParamError> {
        const CTX: &str = "wfd_audio_codecs";
        if value.trim().eq_ignore_ascii_case("none") {
            return Ok(AudioCodecs(Vec::new()));
        }
        let mut codecs = Vec::new();
        for entry in value.split(',').map(str::trim) {
            let fields: Vec<&str> = entry.split_whitespace().collect();
            if fields.len() != 3 {
                return Err(ParamError::FieldCount {
                    context: CTX,
                    expected: 3,
                    found: fields.len(),
                });
            }
            codecs.push(AudioCodec {
                format: fields[0].to_string(),
                modes: hex(CTX, fields[1])?,
                latency: hex(CTX, fields[2])?,
            });
        }
        Ok(AudioCodecs(codecs))
    }

    fn format(&self) -> String {
        if self.0.is_empty() {
            return "none".to_string();
        }
        self.0
            .iter()
            .map(|c| format!("{} {:08x} {:02x}", c.format, c.modes, c.latency))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

// ---------------------------------------------------------------- rtp ports

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientRtpPorts {
    /// The transport profile, kept verbatim (`RTP/AVP/UDP;unicast`).
    pub profile: String,
    pub rtp_port0: u16,
    pub rtp_port1: u16,
    /// The trailing `mode=` value, kept verbatim (`play`).
    pub mode: String,
}

impl WfdParam for ClientRtpPorts {
    fn parse(value: &str) -> Result<Self, ParamError> {
        const CTX: &str = "wfd_client_rtp_ports";
        let fields: Vec<&str> = value.split_whitespace().collect();
        if fields.len() != 4 {
            return Err(ParamError::FieldCount {
                context: CTX,
                expected: 4,
                found: fields.len(),
            });
        }
        // `u16::from_str` accepts a leading '+', and the hex path deliberately
        // rejects a sign, so the decimal path checks the digits itself rather
        // than trusting the parse to refuse "+19000".
        let port = |f: &str| {
            if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ParamError::NotNumeric {
                    context: CTX,
                    field: f.to_string(),
                });
            }
            f.parse::<u16>().map_err(|_| ParamError::NotNumeric {
                context: CTX,
                field: f.to_string(),
            })
        };
        let mode = fields[3]
            .strip_prefix("mode=")
            .ok_or_else(|| ParamError::Shape {
                context: CTX,
                value: fields[3].to_string(),
            })?;
        Ok(ClientRtpPorts {
            profile: fields[0].to_string(),
            rtp_port0: port(fields[1])?,
            rtp_port1: port(fields[2])?,
            mode: mode.to_string(),
        })
    }

    fn format(&self) -> String {
        format!(
            "{} {} {} mode={}",
            self.profile, self.rtp_port0, self.rtp_port1, self.mode
        )
    }
}

// ---------------------------------------------------------------- protection

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentProtection {
    None,
    /// HDCP cannot be supported on Linux (design spec section 6). Parsing it
    /// is how the daemon produces a clear refusal instead of a silent failure.
    Hdcp {
        version: String,
        port: u16,
    },
}

impl ContentProtection {
    pub fn requires_hdcp(&self) -> bool {
        matches!(self, ContentProtection::Hdcp { .. })
    }
}

impl WfdParam for ContentProtection {
    fn parse(value: &str) -> Result<Self, ParamError> {
        const CTX: &str = "wfd_content_protection";
        let trimmed = value.trim();
        if trimmed.eq_ignore_ascii_case("none") {
            return Ok(ContentProtection::None);
        }
        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        if fields.len() != 2 {
            return Err(ParamError::FieldCount {
                context: CTX,
                expected: 2,
                found: fields.len(),
            });
        }
        let digits = fields[1]
            .strip_prefix("port=")
            .ok_or_else(|| ParamError::Shape {
                context: CTX,
                value: fields[1].to_string(),
            })?;
        let not_numeric = || ParamError::NotNumeric {
            context: CTX,
            field: fields[1].to_string(),
        };
        // Same reason as the RTP ports: `u16::from_str` would accept "+1189".
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(not_numeric());
        }
        let port = digits.parse::<u16>().map_err(|_| not_numeric())?;
        Ok(ContentProtection::Hdcp {
            version: fields[0].to_string(),
            port,
        })
    }

    fn format(&self) -> String {
        match self {
            ContentProtection::None => "none".to_string(),
            ContentProtection::Hdcp { version, port } => format!("{version} port={port}"),
        }
    }
}

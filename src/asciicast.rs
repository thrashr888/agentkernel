//! Asciicast v2 format for session recording.
//!
//! Records terminal sessions in the asciicast v2 format for playback with
//! asciinema or similar tools.
//!
//! Format specification: https://github.com/asciinema/asciinema/blob/develop/doc/asciicast-v2.md

// Allow unused items - this is a public API module with functions for future use
#![allow(dead_code)]

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Instant;

/// Asciicast v2 header
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsciicastHeader {
    /// Format version (always 2)
    pub version: u32,
    /// Terminal width in columns
    pub width: u32,
    /// Terminal height in rows
    pub height: u32,
    /// Recording start timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    /// Recording duration in seconds (set on completion)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// Idle time limit in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_time_limit: Option<f64>,
    /// Shell command that was recorded
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Recording title
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Environment variables
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<std::collections::HashMap<String, String>>,
}

impl Default for AsciicastHeader {
    fn default() -> Self {
        Self {
            version: 2,
            width: 80,
            height: 24,
            timestamp: Some(Utc::now().timestamp()),
            duration: None,
            idle_time_limit: None,
            command: None,
            title: None,
            env: None,
        }
    }
}

impl AsciicastHeader {
    /// Create a new header with default terminal size
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a header with specific terminal dimensions
    pub fn with_size(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            ..Default::default()
        }
    }

    /// Set the recording title
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the command being recorded
    pub fn with_command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }
}

/// Event type in an asciicast recording
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    /// Output event (data written to terminal)
    Output,
    /// Input event (data received from terminal)
    Input,
}

impl EventType {
    fn as_str(&self) -> &'static str {
        match self {
            EventType::Output => "o",
            EventType::Input => "i",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "o" => Some(EventType::Output),
            "i" => Some(EventType::Input),
            _ => None,
        }
    }
}

/// A single event in an asciicast recording
#[derive(Debug, Clone)]
pub struct AsciicastEvent {
    /// Time offset in seconds from recording start
    pub time: f64,
    /// Event type (output or input)
    pub event_type: EventType,
    /// Event data (typically terminal output/input)
    pub data: String,
}

impl AsciicastEvent {
    /// Create a new event
    pub fn new(time: f64, event_type: EventType, data: impl Into<String>) -> Self {
        Self {
            time,
            event_type,
            data: data.into(),
        }
    }

    /// Serialize to JSON array format: [time, "o"|"i", "data"]
    pub fn to_json(&self) -> String {
        format!(
            "[{:.6},{:?},{:?}]",
            self.time,
            self.event_type.as_str(),
            self.data
        )
    }

    /// Parse from JSON array format
    pub fn from_json(s: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(s).ok()?;
        let arr = v.as_array()?;
        if arr.len() != 3 {
            return None;
        }

        let time = arr[0].as_f64()?;
        let event_type = EventType::from_str(arr[1].as_str()?)?;
        let data = arr[2].as_str()?.to_string();

        Some(Self {
            time,
            event_type,
            data,
        })
    }
}

/// Asciicast v2 recorder
///
/// Records terminal I/O events with timing information.
pub struct AsciicastRecorder {
    header: AsciicastHeader,
    events: Vec<AsciicastEvent>,
    start_time: Instant,
    output_path: PathBuf,
}

impl AsciicastRecorder {
    /// Create a new recorder with the given output path
    pub fn new(output_path: impl Into<PathBuf>) -> Self {
        Self {
            header: AsciicastHeader::default(),
            events: Vec::new(),
            start_time: Instant::now(),
            output_path: output_path.into(),
        }
    }

    /// Create a recorder with custom header settings
    pub fn with_header(output_path: impl Into<PathBuf>, header: AsciicastHeader) -> Self {
        Self {
            header,
            events: Vec::new(),
            start_time: Instant::now(),
            output_path: output_path.into(),
        }
    }

    /// Record an output event (data written to terminal)
    pub fn record_output(&mut self, data: impl Into<String>) {
        let time = self.start_time.elapsed().as_secs_f64();
        self.events
            .push(AsciicastEvent::new(time, EventType::Output, data));
    }

    /// Record an input event (data received from terminal)
    pub fn record_input(&mut self, data: impl Into<String>) {
        let time = self.start_time.elapsed().as_secs_f64();
        self.events
            .push(AsciicastEvent::new(time, EventType::Input, data));
    }

    /// Get the elapsed time since recording started
    pub fn elapsed(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    /// Finalize and save the recording
    pub fn save(&mut self) -> Result<()> {
        // Update duration
        self.header.duration = Some(self.elapsed());

        let mut file = File::create(&self.output_path)?;

        // Write header as first line
        writeln!(file, "{}", serde_json::to_string(&self.header)?)?;

        // Write events
        for event in &self.events {
            writeln!(file, "{}", event.to_json())?;
        }

        Ok(())
    }

    /// Get the output path
    pub fn path(&self) -> &PathBuf {
        &self.output_path
    }
}

/// Read an asciicast recording from a file
pub fn read_asciicast(path: impl Into<PathBuf>) -> Result<(AsciicastHeader, Vec<AsciicastEvent>)> {
    let file = File::open(path.into())?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    // First line is the header
    let header_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("Empty asciicast file"))??;
    let header: AsciicastHeader = serde_json::from_str(&header_line)?;

    // Remaining lines are events
    let mut events = Vec::new();
    for line in lines {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(event) = AsciicastEvent::from_json(&line) {
            events.push(event);
        }
    }

    Ok((header, events))
}

/// Get the default recording directory
pub fn default_recordings_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agentkernel")
        .join("recordings")
}

/// Generate a recording filename
pub fn generate_recording_name(sandbox_name: &str) -> String {
    let now: DateTime<Utc> = Utc::now();
    format!("{}-{}.cast", sandbox_name, now.format("%Y%m%d-%H%M%S"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_header_serialization() {
        let header = AsciicastHeader::new();
        let json = serde_json::to_string(&header).unwrap();
        assert!(json.contains("\"version\":2"));
        assert!(json.contains("\"width\":80"));
        assert!(json.contains("\"height\":24"));
    }

    #[test]
    fn test_event_serialization() {
        let event = AsciicastEvent::new(1.5, EventType::Output, "hello");
        let json = event.to_json();
        assert!(json.contains("1.5"));
        assert!(json.contains("\"o\""));
        assert!(json.contains("\"hello\""));
    }

    #[test]
    fn test_recorder_save() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.cast");

        let mut recorder = AsciicastRecorder::new(&path);
        recorder.record_output("hello\r\n");
        recorder.record_input("ls\r\n");
        recorder.record_output("file1.txt\r\n");
        recorder.save().unwrap();

        // Verify file exists and has content
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("\"version\":2"));
        assert!(content.contains("hello"));
    }

    #[test]
    fn test_read_asciicast() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.cast");

        // Create a recording
        let mut recorder =
            AsciicastRecorder::with_header(&path, AsciicastHeader::with_size(120, 40));
        recorder.record_output("test output");
        recorder.save().unwrap();

        // Read it back
        let (header, events) = read_asciicast(&path).unwrap();
        assert_eq!(header.width, 120);
        assert_eq!(header.height, 40);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::Output);
    }
}

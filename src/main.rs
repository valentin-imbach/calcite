use chrono::{Datelike, Local, NaiveDate, NaiveTime};
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Terminal,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

#[derive(Parser)]
#[command(name = "calcite")]
#[command(about = "A TUI calendar application", long_about = None)]
struct Args {
    /// Date to select on startup (format: YYYY-MM-DD)
    #[arg(short, long, value_name = "DATE")]
    date: Option<String>,
    
    /// Print a summary of events for the date and exit
    #[arg(short, long)]
    summary: bool,
    
    /// List the next N upcoming events starting from today or specified date
    #[arg(short, long, value_name = "COUNT", default_missing_value = "5", num_args = 0..=1)]
    list: Option<Option<usize>>,
}

fn config_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".config/calcite"))
        .expect("Failed to get home directory")
}

fn ensure_config_dir() -> io::Result<()> {
    let dir = config_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum EventCategory {
    Category1 = 1,
    Category2 = 2,
    Category3 = 3,
    Category4 = 4,
    Category5 = 5,
    Category6 = 6,
    Category7 = 7,
    Category8 = 8,
    Category9 = 9,
}

impl Serialize for EventCategory {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u32(*self as u32)
    }
}

impl<'de> Deserialize<'de> for EventCategory {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u32::deserialize(deserializer)?;
        EventCategory::from_number(value)
            .ok_or_else(|| serde::de::Error::custom(format!("Invalid category: {}", value)))
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct CategoryConfig {
    name: String,
    color: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct IcsCalendar {
    url: String,
    category: u32,
    #[serde(default = "default_export")]
    export: bool,
}

fn default_export() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    categories: Vec<CategoryConfig>,
    #[serde(default)]
    ics_calendars: Vec<IcsCalendar>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    export_path: Option<String>,
}

impl Config {
    fn load() -> Self {
        let config_path = config_dir().join("config.toml");
        if config_path.exists() {
            if let Ok(content) = fs::read_to_string(&config_path) {
                if let Ok(config) = toml::from_str::<Config>(&content) {
                    return config;
                }
            }
        }
        
        // Return default config
        Self::default()
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            categories: vec![
                CategoryConfig { name: "Work".to_string(), color: "Blue".to_string() },
                CategoryConfig { name: "Personal".to_string(), color: "Green".to_string() },
                CategoryConfig { name: "Health".to_string(), color: "Red".to_string() },
                CategoryConfig { name: "Social".to_string(), color: "Magenta".to_string() },
                CategoryConfig { name: "Finance".to_string(), color: "Yellow".to_string() },
                CategoryConfig { name: "Education".to_string(), color: "Cyan".to_string() },
                CategoryConfig { name: "Travel".to_string(), color: "LightBlue".to_string() },
                CategoryConfig { name: "Shopping".to_string(), color: "LightGreen".to_string() },
                CategoryConfig { name: "Other".to_string(), color: "Gray".to_string() },
            ],
            ics_calendars: vec![],
            min_year: None,
            max_year: None,
            export_path: None,
        }
    }
}

fn fetch_ics_events(config: &Config) -> Vec<(CalendarEvent, bool)> {
    let mut external_events = Vec::new();
    
    for ics_calendar in &config.ics_calendars {
        if let Some(category) = EventCategory::from_number(ics_calendar.category) {
            if let Ok(response) = reqwest::blocking::get(&ics_calendar.url) {
                if let Ok(content) = response.text() {
                    if let Ok(events) = parse_ics_content(&content, category, config.min_year, config.max_year) {
                        // Tag each event with the export flag from its calendar
                        for event in events {
                            external_events.push((event, ics_calendar.export));
                        }
                    }
                }
            }
        }
    }
    
    external_events
}

fn parse_ics_content(content: &str, category: EventCategory, min_year: Option<i32>, max_year: Option<i32>) -> Result<Vec<CalendarEvent>, Box<dyn std::error::Error>> {
    let reader = ical::IcalParser::new(content.as_bytes());
    let mut events = Vec::new();
    
    for calendar in reader {
        let calendar = calendar?;
        for component in calendar.events {
            let mut name = String::new();
            let mut dtstart: Option<NaiveDate> = None;
            let mut dtend: Option<NaiveDate> = None;
            let mut start_time: Option<NaiveTime> = None;
            let mut end_time: Option<NaiveTime> = None;
            
            for property in component.properties {
                match property.name.as_str() {
                    "SUMMARY" => {
                        if let Some(value) = property.value {
                            name = value;
                        }
                    }
                    "DTSTART" => {
                        if let Some(value) = property.value {
                            // Try to parse as datetime first, then as date
                            if let Some(parsed) = parse_ical_datetime(&value) {
                                dtstart = Some(parsed.0);
                                start_time = parsed.1;
                            }
                        }
                    }
                    "DTEND" => {
                        if let Some(value) = property.value {
                            if let Some(parsed) = parse_ical_datetime(&value) {
                                dtend = Some(parsed.0);
                                end_time = parsed.1;
                            }
                        }
                    }
                    _ => {}
                }
            }
            
            if !name.is_empty() && dtstart.is_some() {
                let date = dtstart.unwrap();
                let event_year = date.year();
                
                // Filter by year range if specified
                let in_range = match (min_year, max_year) {
                    (Some(min), Some(max)) => event_year >= min && event_year <= max,
                    (Some(min), None) => event_year >= min,
                    (None, Some(max)) => event_year <= max,
                    (None, None) => true, // No filtering
                };
                
                if in_range {
                    // For all-day events, DTEND is exclusive in ICS format
                    // Subtract one day if dtend exists and there's no time component
                    let adjusted_end_date = if let Some(end_date) = dtend {
                        if end_time.is_none() && start_time.is_none() {
                            // All-day event: DTEND is exclusive, so subtract 1 day
                            Some(end_date - chrono::Days::new(1))
                        } else {
                            // Timed event: DTEND is inclusive
                            Some(end_date)
                        }
                    } else {
                        None
                    };
                    
                    let event = CalendarEvent {
                        name,
                        date,
                        end_date: adjusted_end_date,
                        time: start_time,
                        end_time,
                        category,
                        repeat: RepeatInterval::Daily,
                        number: None,
                    };
                    events.push(event);
                }
            }
        }
    }
    
    Ok(events)
}

fn parse_ical_datetime(value: &str) -> Option<(NaiveDate, Option<NaiveTime>)> {
    // Format: YYYYMMDD or YYYYMMDDTHHMMSS or YYYYMMDDTHHMMSSZ
    let cleaned = value.replace("Z", "").replace("-", "").replace(":", "");
    
    if cleaned.len() >= 8 {
        let year = cleaned[0..4].parse::<i32>().ok()?;
        let month = cleaned[4..6].parse::<u32>().ok()?;
        let day = cleaned[6..8].parse::<u32>().ok()?;
        
        let date = NaiveDate::from_ymd_opt(year, month, day)?;
        
        // Check if there's a time component
        if cleaned.len() >= 15 && cleaned.chars().nth(8) == Some('T') {
            let hour = cleaned[9..11].parse::<u32>().ok()?;
            let minute = cleaned[11..13].parse::<u32>().ok()?;
            let second = cleaned[13..15].parse::<u32>().ok()?;
            
            let time = NaiveTime::from_hms_opt(hour, minute, second)?;
            Some((date, Some(time)))
        } else {
            Some((date, None))
        }
    } else {
        None
    }
}

fn parse_color(color_str: &str) -> Color {
    match color_str {
        "Black" => Color::Black,
        "Red" => Color::Red,
        "Green" => Color::Green,
        "Yellow" => Color::Yellow,
        "Blue" => Color::Blue,
        "Magenta" => Color::Magenta,
        "Cyan" => Color::Cyan,
        "Gray" | "Grey" => Color::Gray,
        "DarkGray" | "DarkGrey" => Color::DarkGray,
        "LightRed" => Color::LightRed,
        "LightGreen" => Color::LightGreen,
        "LightYellow" => Color::LightYellow,
        "LightBlue" => Color::LightBlue,
        "LightMagenta" => Color::LightMagenta,
        "LightCyan" => Color::LightCyan,
        "White" => Color::White,
        "Orange" => Color::Rgb(255, 165, 0),
        "Brown" => Color::Rgb(165, 42, 42),
        "Teal" => Color::Rgb(0, 128, 128),
        _ => Color::White, // fallback
    }
}

impl EventCategory {
    fn color(&self, config: &Config) -> Color {
        let index = (*self as usize) - 1;
        if index < config.categories.len() {
            parse_color(&config.categories[index].color)
        } else {
            Color::White
        }
    }
    
    fn name<'a>(&self, config: &'a Config) -> &'a str {
        let index = (*self as usize) - 1;
        if index < config.categories.len() {
            &config.categories[index].name
        } else {
            "Unknown"
        }
    }
    
    fn from_number(n: u32) -> Option<Self> {
        match n {
            1 => Some(EventCategory::Category1),
            2 => Some(EventCategory::Category2),
            3 => Some(EventCategory::Category3),
            4 => Some(EventCategory::Category4),
            5 => Some(EventCategory::Category5),
            6 => Some(EventCategory::Category6),
            7 => Some(EventCategory::Category7),
            8 => Some(EventCategory::Category8),
            9 => Some(EventCategory::Category9),
            _ => None,
        }
    }
    
    fn all() -> [EventCategory; 9] {
        [
            EventCategory::Category1,
            EventCategory::Category2,
            EventCategory::Category3,
            EventCategory::Category4,
            EventCategory::Category5,
            EventCategory::Category6,
            EventCategory::Category7,
            EventCategory::Category8,
            EventCategory::Category9,
        ]
    }
}

impl Default for EventCategory {
    fn default() -> Self {
        EventCategory::Category1
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
enum RepeatInterval {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Default for RepeatInterval {
    fn default() -> Self {
        RepeatInterval::Daily
    }
}

impl RepeatInterval {
    fn to_string(&self) -> String {
        match self {
            RepeatInterval::Daily => "Daily".to_string(),
            RepeatInterval::Weekly => "Weekly".to_string(),
            RepeatInterval::Monthly => "Monthly".to_string(),
            RepeatInterval::Yearly => "Yearly".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct CalendarEvent {
    name: String,
    date: NaiveDate,
    end_date: Option<NaiveDate>,
    time: Option<NaiveTime>,
    end_time: Option<NaiveTime>,
    #[serde(default)]
    category: EventCategory,
    #[serde(default, skip_serializing_if = "is_default_repeat")]
    repeat: RepeatInterval,
    #[serde(skip_serializing_if = "Option::is_none")]
    number: Option<u32>,
}

fn is_default_repeat(repeat: &RepeatInterval) -> bool {
    *repeat == RepeatInterval::Daily
}

#[derive(Serialize, Deserialize)]
struct EventsFile {
    events: Vec<CalendarEvent>,
}

#[derive(Clone, Copy)]
enum EventSource {
    Local(usize),    // Index into local_events
    External(usize), // Index into external_events
}

struct App {
    selected_date: NaiveDate,
    current_month: u32,
    current_year: i32,
    days_in_month: u32,
    first_weekday: u32,
    input_mode: InputMode,
    event_form: EventForm,
    events: HashMap<NaiveDate, Vec<EventSource>>, // References to local or external events
    local_events: Vec<CalendarEvent>,
    external_events: Vec<CalendarEvent>,
    external_events_exportable: Vec<bool>, // Tracks which external events should be exported
    show_event_numbers: bool,
    show_delete_numbers: bool,
    config: Config,
    search_state: SearchState,
}

#[derive(PartialEq)]
enum InputMode {
    Normal,
    EventPopup,
    DeletingEvent(usize), // Index of event to delete (showing confirmation)
    ShowingInfo,
    Searching,
}

#[derive(PartialEq, Clone, Copy)]
enum SearchMode {
    Future,
    Past,
    All,
}

impl SearchMode {
    fn next(&self) -> Self {
        match self {
            SearchMode::Future => SearchMode::Past,
            SearchMode::Past => SearchMode::All,
            SearchMode::All => SearchMode::Future,
        }
    }
    
    fn to_string(&self) -> &str {
        match self {
            SearchMode::Future => "Future",
            SearchMode::Past => "Past",
            SearchMode::All => "All",
        }
    }
}

struct SearchState {
    query: String,
    results: Vec<SearchResult>,
    selected_index: usize,
    mode: SearchMode,
}

struct SearchResult {
    event_name: String,
    date: NaiveDate,
    time: Option<NaiveTime>,
    source: EventSource,
    score: i64,
}

impl SearchState {
    fn new() -> Self {
        SearchState {
            query: String::new(),
            results: Vec::new(),
            selected_index: 0,
            mode: SearchMode::Future,
        }
    }
    
    fn cycle_mode(&mut self) {
        self.mode = self.mode.next();
        self.selected_index = 0;
    }
    
    fn update_results(&mut self, local_events: &[CalendarEvent], external_events: &[CalendarEvent], config: &Config) {
        self.results.clear();
        self.selected_index = 0;
        
        if self.query.is_empty() {
            return;
        }
        
        let matcher = SkimMatcherV2::default();
        let now = Local::now().date_naive();
        
        // Search through local events
        for (idx, event) in local_events.iter().enumerate() {
            // Filter by mode first
            let include = match self.mode {
                SearchMode::Future => event.date >= now,
                SearchMode::Past => event.date < now,
                SearchMode::All => true,
            };
            
            if !include {
                continue;
            }
            
            // Build searchable text with metadata
            let mut searchable_text = event.name.clone();
            
            // Add category name
            searchable_text.push(' ');
            searchable_text.push_str(event.category.name(config));
            
            // Add date
            searchable_text.push(' ');
            searchable_text.push_str(&event.date.format("%Y-%m-%d").to_string());
            searchable_text.push(' ');
            searchable_text.push_str(&event.date.format("%Y %m %d").to_string());
            
            // Add time if present
            if let Some(time) = event.time {
                searchable_text.push(' ');
                searchable_text.push_str(&time.format("%H:%M").to_string());
                searchable_text.push(' ');
                searchable_text.push_str(&time.format("%H %M").to_string());
            }
            
            // Fuzzy match
            if let Some(score) = matcher.fuzzy_match(&searchable_text, &self.query) {
                self.results.push(SearchResult {
                    event_name: event.name.clone(),
                    date: event.date,
                    time: event.time,
                    source: EventSource::Local(idx),
                    score,
                });
            }
        }
        
        // Search through external events
        for (idx, event) in external_events.iter().enumerate() {
            // Filter by mode first
            let include = match self.mode {
                SearchMode::Future => event.date >= now,
                SearchMode::Past => event.date < now,
                SearchMode::All => true,
            };
            
            if !include {
                continue;
            }
            
            // Build searchable text with metadata
            let mut searchable_text = event.name.clone();
            
            // Add category name
            searchable_text.push(' ');
            searchable_text.push_str(event.category.name(config));
            
            // Add date
            searchable_text.push(' ');
            searchable_text.push_str(&event.date.format("%Y-%m-%d").to_string());
            searchable_text.push(' ');
            searchable_text.push_str(&event.date.format("%Y %m %d").to_string());
            
            // Add time if present
            if let Some(time) = event.time {
                searchable_text.push(' ');
                searchable_text.push_str(&time.format("%H:%M").to_string());
                searchable_text.push(' ');
                searchable_text.push_str(&time.format("%H %M").to_string());
            }
            
            // Fuzzy match
            if let Some(score) = matcher.fuzzy_match(&searchable_text, &self.query) {
                self.results.push(SearchResult {
                    event_name: event.name.clone(),
                    date: event.date,
                    time: event.time,
                    source: EventSource::External(idx),
                    score,
                });
            }
        }
        
        // Sort results by score (descending), then by date/time
        self.results.sort_by(|a, b| {
            let score_cmp = b.score.cmp(&a.score); // Higher score first
            if score_cmp != std::cmp::Ordering::Equal {
                score_cmp
            } else {
                let date_cmp = a.date.cmp(&b.date);
                if date_cmp != std::cmp::Ordering::Equal {
                    date_cmp
                } else {
                    // Within same date, sort by time (None comes first)
                    match (a.time, b.time) {
                        (None, Some(_)) => std::cmp::Ordering::Less,
                        (Some(_), None) => std::cmp::Ordering::Greater,
                        (Some(t1), Some(t2)) => t1.cmp(&t2),
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                }
            }
        });
        
        // For past events, keep score-based sorting but reverse date order within same score
        if self.mode == SearchMode::Past {
            self.results.sort_by(|a, b| {
                let score_cmp = b.score.cmp(&a.score);
                if score_cmp != std::cmp::Ordering::Equal {
                    score_cmp
                } else {
                    // Reverse date order for past events
                    let date_cmp = b.date.cmp(&a.date);
                    if date_cmp != std::cmp::Ordering::Equal {
                        date_cmp
                    } else {
                        match (b.time, a.time) {
                            (None, Some(_)) => std::cmp::Ordering::Less,
                            (Some(_), None) => std::cmp::Ordering::Greater,
                            (Some(t1), Some(t2)) => t1.cmp(&t2),
                            (None, None) => std::cmp::Ordering::Equal,
                        }
                    }
                }
            });
        }
    }
    
    fn move_selection(&mut self, delta: i32) {
        if self.results.is_empty() {
            return;
        }
        
        let new_index = self.selected_index as i32 + delta;
        if new_index >= 0 && new_index < self.results.len() as i32 {
            self.selected_index = new_index as usize;
        }
    }
    
    fn get_selected_date(&self) -> Option<NaiveDate> {
        self.results.get(self.selected_index).map(|r| r.date)
    }
}

struct EventForm {
    fields: Vec<FormField>,
    current_field_idx: usize,
    category: EventCategory,
    repeat: RepeatInterval,
    editing_index: Option<usize>, // None for new event, Some(index) for editing
}

enum FieldType {
    Text,
    Date,
    Time,
    Number,
}

struct FormField {
    label: String,
    value: String,
    required: bool,
    placeholder: String,
    field_type: FieldType,
}

impl FormField {
    fn new(label: &str, value: String, required: bool, placeholder: &str, field_type: FieldType) -> Self {
        FormField {
            label: label.to_string(),
            value,
            required,
            placeholder: placeholder.to_string(),
            field_type,
        }
    }
    
    fn validate(&self) -> Result<(), String> {
        if self.required && self.value.trim().is_empty() {
            return Err(format!("{} is required", self.label));
        }
        
        // Validate format completeness for structured fields
        if !self.value.is_empty() {
            match self.field_type {
                FieldType::Date => {
                    if self.value.len() != 10 {
                        return Err(format!("{} must be in format YYYY-MM-DD", self.label));
                    }
                    // Validate that the date is actually valid
                    if NaiveDate::parse_from_str(&self.value, "%Y-%m-%d").is_err() {
                        return Err(format!("{} is not a valid date", self.label));
                    }
                }
                FieldType::Time => {
                    if self.value.len() != 5 {
                        return Err(format!("{} must be in format HH:MM", self.label));
                    }
                }
                _ => {}
            }
        }
        
        Ok(())
    }
    
    fn can_accept_char(&self, c: char, position: usize) -> bool {
        match self.field_type {
            FieldType::Text => true,
            FieldType::Number => c.is_ascii_digit(),
            FieldType::Date => {
                // Format: YYYY-MM-DD (10 chars)
                match position {
                    0..=3 => c.is_ascii_digit(), // YYYY
                    4 | 7 => c == '-',            // dashes
                    5 => c == '0' || c == '1', // Month first digit (01-12)
                    8 => c == '0' || c == '1' || c == '2' || c == '3', // Day first digit (01-31)
                    6 => {
                        // Second digit of month (01-12)
                        if self.value.len() >= 6 {
                            let first = self.value.chars().nth(5).unwrap_or('0');
                            if first == '0' {
                                c.is_ascii_digit() && c != '0' // 01-09
                            } else if first == '1' {
                                c == '0' || c == '1' || c == '2' // 10-12
                            } else {
                                false
                            }
                        } else {
                            c.is_ascii_digit()
                        }
                    }
                    9 => {
                        // Second digit of day (01-31)
                        if self.value.len() >= 9 {
                            let first = self.value.chars().nth(8).unwrap_or('0');
                            if first == '0' {
                                c.is_ascii_digit() && c != '0' // 01-09
                            } else if first == '1' || first == '2' {
                                c.is_ascii_digit() // 10-29
                            } else if first == '3' {
                                c == '0' || c == '1' // 30-31
                            } else {
                                false
                            }
                        } else {
                            c.is_ascii_digit()
                        }
                    }
                    _ => false,
                }
            }
            FieldType::Time => {
                // Format: HH:MM (5 chars)
                match position {
                    0 => c == '0' || c == '1' || c == '2', // 00-23
                    1 => {
                        // Second digit of hour
                        if self.value.len() >= 1 {
                            let first = self.value.chars().nth(0).unwrap_or('0');
                            if first == '0' || first == '1' {
                                c.is_ascii_digit() // 00-19
                            } else if first == '2' {
                                c == '0' || c == '1' || c == '2' || c == '3' // 20-23
                            } else {
                                false
                            }
                        } else {
                            c.is_ascii_digit()
                        }
                    }
                    2 => c == ':', // colon
                    3 => c >= '0' && c <= '5', // 00-59
                    4 => c.is_ascii_digit(),
                    _ => false,
                }
            }
        }
    }
}

impl EventForm {
    fn new(date: NaiveDate) -> Self {
        EventForm {
            fields: vec![
                FormField::new("Name", String::new(), true, "Event name", FieldType::Text),
                FormField::new("Date", date.format("%Y-%m-%d").to_string(), true, "YYYY-MM-DD", FieldType::Date),
                FormField::new("End Date", String::new(), false, "YYYY-MM-DD", FieldType::Date),
                FormField::new("Time", String::new(), false, "HH:MM", FieldType::Time),
                FormField::new("End Time", String::new(), false, "HH:MM", FieldType::Time),
                FormField::new("Repeat", String::new(), false, "Press D/W/M/Y", FieldType::Text),
                FormField::new("Number", String::new(), false, "Number", FieldType::Number),
                FormField::new("Category", String::new(), false, "Press 1-9", FieldType::Text),
            ],
            current_field_idx: 0,
            category: EventCategory::default(),
            repeat: RepeatInterval::default(),
            editing_index: None,
        }
    }
    
    fn from_event(event: &CalendarEvent, index: usize) -> Self {
        EventForm {
            fields: vec![
                FormField::new("Name", event.name.clone(), true, "Event name", FieldType::Text),
                FormField::new("Date", event.date.format("%Y-%m-%d").to_string(), true, "YYYY-MM-DD", FieldType::Date),
                FormField::new("End Date", event.end_date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default(), false, "YYYY-MM-DD", FieldType::Date),
                FormField::new("Time", event.time.map(|t| t.format("%H:%M").to_string()).unwrap_or_default(), false, "HH:MM", FieldType::Time),
                FormField::new("End Time", event.end_time.map(|t| t.format("%H:%M").to_string()).unwrap_or_default(), false, "HH:MM", FieldType::Time),
                FormField::new("Repeat", String::new(), false, "Press D/W/M/Y", FieldType::Text),
                FormField::new("Number", event.number.map(|n| n.to_string()).unwrap_or_default(), false, "Number", FieldType::Number),
                FormField::new("Category", String::new(), false, "Press 1-9", FieldType::Text),
            ],
            current_field_idx: 0,
            category: event.category,
            repeat: event.repeat.clone(),
            editing_index: Some(index),
        }
    }

    fn current_field_mut(&mut self) -> &mut FormField {
        // Don't allow mutation of the category/repeat pseudo-fields (index 5, 7)
        if self.current_field_idx == 5 || self.current_field_idx == 7 {
            &mut self.fields[0] // Return dummy field
        } else {
            &mut self.fields[self.current_field_idx]
        }
    }

    fn move_field(&mut self, dx: i32, dy: i32) {
        // Grid layout: Name (full width), then 2x2 grid, then Repeat+Number, then Category
        let new_idx = match (self.current_field_idx, dx, dy) {
            (0, 0, 1) => 1,  // Name -> Start Date
            (0, _, _) => 0,
            
            (1, 1, 0) => 2,  // Start Date -> End Date
            (1, 0, -1) => 0, // Start Date -> Name
            (1, 0, 1) => 3,  // Start Date -> Start Time
            (1, _, _) => 1,
            
            (2, -1, 0) => 1, // End Date -> Start Date
            (2, 0, -1) => 0, // End Date -> Name
            (2, 0, 1) => 4,  // End Date -> End Time
            (2, _, _) => 2,
            
            (3, 1, 0) => 4,  // Start Time -> End Time
            (3, 0, -1) => 1, // Start Time -> Start Date
            (3, 0, 1) => 5,  // Start Time -> Repeat
            (3, _, _) => 3,
            
            (4, -1, 0) => 3, // End Time -> Start Time
            (4, 0, -1) => 2, // End Time -> End Date
            (4, 0, 1) => 6,  // End Time -> Number
            (4, _, _) => 4,
            
            (5, 1, 0) => 6,  // Repeat -> Number
            (5, 0, -1) => 3, // Repeat -> Start Time
            (5, 0, 1) => 7,  // Repeat -> Category
            (5, _, _) => 5,
            
            (6, -1, 0) => 5, // Number -> Repeat
            (6, 0, -1) => 4, // Number -> End Time
            (6, 0, 1) => 7,  // Number -> Category
            (6, _, _) => 6,
            
            (7, 0, -1) => 5, // Category -> Repeat
            (7, _, _) => 7,
            
            _ => self.current_field_idx,
        };
        
        self.current_field_idx = new_idx;
    }

    fn next_field(&mut self) {
        self.current_field_idx = (self.current_field_idx + 1) % self.fields.len();
    }

    fn prev_field(&mut self) {
        self.current_field_idx = (self.current_field_idx + self.fields.len() - 1) % self.fields.len();
    }

    fn validate_and_create_event(&self) -> Result<CalendarEvent, String> {
        // Collect all errors
        if let Some(error) = self.get_first_error() {
            return Err(error);
        }

        // Parse date
        let date = NaiveDate::parse_from_str(&self.fields[1].value, "%Y-%m-%d")
            .map_err(|_| "Invalid date format".to_string())?;

        // Parse optional end date
        let end_date = if !self.fields[2].value.is_empty() {
            Some(NaiveDate::parse_from_str(&self.fields[2].value, "%Y-%m-%d")
                .map_err(|_| "Invalid end date format".to_string())?)
        } else {
            None
        };

        // Parse optional time
        let time = if !self.fields[3].value.is_empty() {
            Some(NaiveTime::parse_from_str(&self.fields[3].value, "%H:%M")
                .map_err(|_| "Invalid time format".to_string())?)
        } else {
            None
        };

        // Parse optional end time
        let end_time = if !self.fields[4].value.is_empty() {
            Some(NaiveTime::parse_from_str(&self.fields[4].value, "%H:%M")
                .map_err(|_| "Invalid end time format".to_string())?)
        } else {
            None
        };

        // Parse optional number (field 6)
        let number = if !self.fields[6].value.is_empty() {
            Some(self.fields[6].value.parse::<u32>()
                .map_err(|_| "Invalid number format".to_string())?)
        } else {
            None
        };

        Ok(CalendarEvent {
            name: self.fields[0].value.trim().to_string(),
            date,
            end_date,
            time,
            end_time,
            category: self.category,
            repeat: self.repeat.clone(),
            number,
        })
    }
    
    fn get_first_error(&self) -> Option<String> {
        // Check all fields in order and return the first error found
        for i in 0..self.fields.len() {
            if let Some(error) = self.get_field_error(i) {
                return Some(error);
            }
        }
        None
    }
    
    fn get_field_error(&self, field_idx: usize) -> Option<String> {
        // Check mutual exclusion for end_date (2) and number (6)
        if field_idx == 2 || field_idx == 6 {
            let end_date_filled = !self.fields[2].value.is_empty();
            let number_filled = !self.fields[6].value.is_empty();
            
            if end_date_filled && number_filled {
                return Some("Cannot set both End Date and Number".to_string());
            }
        }
        
        // Check basic validation (required fields)
        if let Err(e) = self.fields[field_idx].validate() {
            return Some(e);
        }
        
        // Check format validation
        match field_idx {
            0 => {
                // Name field - just check if empty (already checked above)
                None
            }
            1 => {
                // Date field
                if !self.fields[1].value.is_empty() {
                    NaiveDate::parse_from_str(&self.fields[1].value, "%Y-%m-%d")
                        .err()
                        .map(|_| "Date: Invalid date".to_string())
                } else {
                    None
                }
            }
            2 => {
                // End Date field
                if !self.fields[2].value.is_empty() {
                    // Check if end date is valid
                    if let Err(_) = NaiveDate::parse_from_str(&self.fields[2].value, "%Y-%m-%d") {
                        return Some("End Date: Invalid date".to_string());
                    }
                    
                    // Check if end date is after start date
                    if !self.fields[1].value.is_empty() {
                        if let (Ok(start), Ok(end)) = (
                            NaiveDate::parse_from_str(&self.fields[1].value, "%Y-%m-%d"),
                            NaiveDate::parse_from_str(&self.fields[2].value, "%Y-%m-%d")
                        ) {
                            if end < start {
                                return Some("End Date: Must be after start date".to_string());
                            }
                        }
                    }
                }
                None
            }
            3 => {
                // Time field
                if !self.fields[3].value.is_empty() {
                    NaiveTime::parse_from_str(&self.fields[3].value, "%H:%M")
                        .err()
                        .map(|_| "Time: Invalid time".to_string())
                } else {
                    None
                }
            }
            4 => {
                // End Time field
                if !self.fields[4].value.is_empty() {
                    // Check if time is set (end_time requires time)
                    if self.fields[3].value.is_empty() {
                        return Some("End Time: Requires Time to be set".to_string());
                    }
                    
                    // Check if end time is valid
                    if let Err(_) = NaiveTime::parse_from_str(&self.fields[4].value, "%H:%M") {
                        return Some("End Time: Invalid time".to_string());
                    }
                    
                    // Check if end time is after start time
                    if let (Ok(start), Ok(end)) = (
                        NaiveTime::parse_from_str(&self.fields[3].value, "%H:%M"),
                        NaiveTime::parse_from_str(&self.fields[4].value, "%H:%M")
                    ) {
                        if end <= start {
                            return Some("End Time: Must be after start time".to_string());
                        }
                    }
                }
                None
            }
            6 => {
                // Number field
                if !self.fields[6].value.is_empty() {
                    self.fields[6].value.parse::<u32>()
                        .err()
                        .map(|_| "Number: Must be a positive integer".to_string())
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl App {
    fn with_date(date: NaiveDate) -> App {
        let year = date.year();
        let month = date.month();
        
        let first_day = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
        let first_weekday = first_day.weekday().num_days_from_monday();
        
        let days_in_month = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap()
        }
        .signed_duration_since(first_day)
        .num_days() as u32;
        
        let config = Config::load();
        
        App {
            selected_date: date,
            current_month: month,
            current_year: year,
            days_in_month,
            first_weekday,
            input_mode: InputMode::Normal,
            event_form: EventForm::new(date),
            events: HashMap::new(),
            local_events: Vec::new(),
            external_events: Vec::new(),
            external_events_exportable: Vec::new(),
            show_event_numbers: false,
            show_delete_numbers: false,
            config,
            search_state: SearchState::new(),
        }
    }
    
    fn export_to_ics(&self) -> Result<(), String> {
        let mut ics_content = String::new();
        
        // ICS header
        ics_content.push_str("BEGIN:VCALENDAR\r\n");
        ics_content.push_str("VERSION:2.0\r\n");
        ics_content.push_str("PRODID:-//calcite//calcite TUI Calendar//EN\r\n");
        ics_content.push_str("CALSCALE:GREGORIAN\r\n");
        
        // Export each local event
        for (idx, event) in self.local_events.iter().enumerate() {
            ics_content.push_str("BEGIN:VEVENT\r\n");
            
            // UID (unique identifier)
            let uid = format!("calcite-{}-{}@localhost", event.date.format("%Y%m%d"), idx);
            ics_content.push_str(&format!("UID:{}\r\n", uid));
            
            // SUMMARY (event name)
            ics_content.push_str(&format!("SUMMARY:{}\r\n", event.name));
            
            // DTSTART (start date/time)
            if let Some(time) = event.time {
                let dtstart = format!("{}T{}", 
                    event.date.format("%Y%m%d"),
                    time.format("%H%M%S"));
                ics_content.push_str(&format!("DTSTART:{}\r\n", dtstart));
            } else {
                // All-day event
                let dtstart = event.date.format("%Y%m%d");
                ics_content.push_str(&format!("DTSTART;VALUE=DATE:{}\r\n", dtstart));
            }
            
            // DTEND (end date/time)
            if let Some(end_time) = event.end_time {
                if event.time.is_some() {
                    let dtend = format!("{}T{}", 
                        event.date.format("%Y%m%d"),
                        end_time.format("%H%M%S"));
                    ics_content.push_str(&format!("DTEND:{}\r\n", dtend));
                }
            } else if event.time.is_none() {
                // All-day event - end date is next day
                let end_date = event.date.succ_opt().unwrap_or(event.date);
                ics_content.push_str(&format!("DTEND;VALUE=DATE:{}\r\n", end_date.format("%Y%m%d")));
            }
            
            // Handle recurrence
            if let Some(number) = event.number {
                if number > 1 {
                    // Use RRULE with COUNT
                    let freq = match event.repeat {
                        RepeatInterval::Daily => "DAILY",
                        RepeatInterval::Weekly => "WEEKLY",
                        RepeatInterval::Monthly => "MONTHLY",
                        RepeatInterval::Yearly => "YEARLY",
                    };
                    ics_content.push_str(&format!("RRULE:FREQ={};COUNT={}\r\n", freq, number));
                }
            } else if let Some(end_date) = event.end_date {
                // Use RRULE with UNTIL
                let freq = match event.repeat {
                    RepeatInterval::Daily => "DAILY",
                    RepeatInterval::Weekly => "WEEKLY",
                    RepeatInterval::Monthly => "MONTHLY",
                    RepeatInterval::Yearly => "YEARLY",
                };
                let until = if event.time.is_some() {
                    format!("{}T235959", end_date.format("%Y%m%d"))
                } else {
                    end_date.format("%Y%m%d").to_string()
                };
                ics_content.push_str(&format!("RRULE:FREQ={};UNTIL={}\r\n", freq, until));
            }
            
            // CATEGORIES (using category name from config)
            let category_name = event.category.name(&self.config);
            ics_content.push_str(&format!("CATEGORIES:{}\r\n", category_name));
            
            // DTSTAMP (timestamp of creation - use current time)
            let now = Local::now();
            let dtstamp = now.format("%Y%m%dT%H%M%SZ");
            ics_content.push_str(&format!("DTSTAMP:{}\r\n", dtstamp));
            
            ics_content.push_str("END:VEVENT\r\n");
        }
        
        // Export external events that are marked as exportable
        for (idx, (event, exportable)) in self.external_events.iter().zip(&self.external_events_exportable).enumerate() {
            if !*exportable {
                continue;
            }
            
            ics_content.push_str("BEGIN:VEVENT\r\n");
            
            // UID (unique identifier for external events)
            let uid = format!("calcite-ext-{}-{}@localhost", event.date.format("%Y%m%d"), idx);
            ics_content.push_str(&format!("UID:{}\r\n", uid));
            
            // SUMMARY (event name)
            ics_content.push_str(&format!("SUMMARY:{}\r\n", event.name));
            
            // DTSTART (start date/time)
            if let Some(time) = event.time {
                let dtstart = format!("{}T{}", 
                    event.date.format("%Y%m%d"),
                    time.format("%H%M%S"));
                ics_content.push_str(&format!("DTSTART:{}\r\n", dtstart));
            } else {
                // All-day event
                let dtstart = event.date.format("%Y%m%d");
                ics_content.push_str(&format!("DTSTART;VALUE=DATE:{}\r\n", dtstart));
            }
            
            // DTEND (end date/time)
            if let Some(end_time) = event.end_time {
                if event.time.is_some() {
                    let dtend = format!("{}T{}", 
                        event.date.format("%Y%m%d"),
                        end_time.format("%H%M%S"));
                    ics_content.push_str(&format!("DTEND:{}\r\n", dtend));
                }
            } else if event.time.is_none() {
                // All-day event - end date is next day
                let end_date = event.date.succ_opt().unwrap_or(event.date);
                ics_content.push_str(&format!("DTEND;VALUE=DATE:{}\r\n", end_date.format("%Y%m%d")));
            }
            
            // Handle recurrence
            if let Some(number) = event.number {
                if number > 1 {
                    // Use RRULE with COUNT
                    let freq = match event.repeat {
                        RepeatInterval::Daily => "DAILY",
                        RepeatInterval::Weekly => "WEEKLY",
                        RepeatInterval::Monthly => "MONTHLY",
                        RepeatInterval::Yearly => "YEARLY",
                    };
                    ics_content.push_str(&format!("RRULE:FREQ={};COUNT={}\r\n", freq, number));
                }
            } else if let Some(end_date) = event.end_date {
                // Use RRULE with UNTIL
                let freq = match event.repeat {
                    RepeatInterval::Daily => "DAILY",
                    RepeatInterval::Weekly => "WEEKLY",
                    RepeatInterval::Monthly => "MONTHLY",
                    RepeatInterval::Yearly => "YEARLY",
                };
                let until = if event.time.is_some() {
                    format!("{}T235959", end_date.format("%Y%m%d"))
                } else {
                    end_date.format("%Y%m%d").to_string()
                };
                ics_content.push_str(&format!("RRULE:FREQ={};UNTIL={}\r\n", freq, until));
            }
            
            // CATEGORIES (using category name from config)
            let category_name = event.category.name(&self.config);
            ics_content.push_str(&format!("CATEGORIES:{}\r\n", category_name));
            
            // DTSTAMP (timestamp of creation - use current time)
            let now = Local::now();
            let dtstamp = now.format("%Y%m%dT%H%M%SZ");
            ics_content.push_str(&format!("DTSTAMP:{}\r\n", dtstamp));
            
            ics_content.push_str("END:VEVENT\r\n");
        }
        
        // ICS footer
        ics_content.push_str("END:VCALENDAR\r\n");
        
        // Determine export path with timestamp formatting
        let export_path = if let Some(path) = &self.config.export_path {
            let path_str = path.as_str();
            // Expand ~ to home directory
            let expanded = if path_str.starts_with("~/") {
                if let Some(home) = std::env::var_os("HOME") {
                    path_str.replacen("~", &home.to_string_lossy(), 1)
                } else {
                    path_str.to_string()
                }
            } else if path_str == "~" {
                if let Some(home) = std::env::var_os("HOME") {
                    home.to_string_lossy().to_string()
                } else {
                    path_str.to_string()
                }
            } else if PathBuf::from(path_str).is_absolute() {
                path_str.to_string()
            } else {
                // Relative path - relative to config directory
                config_dir().join(path_str).to_string_lossy().to_string()
            };
            
            // Apply timestamp formatting
            let now = Local::now();
            PathBuf::from(now.format(&expanded).to_string())
        } else {
            // Default: calendar.ics in config directory
            config_dir().join("calendar.ics")
        };
        
        // Validate that the parent directory exists
        if let Some(parent) = export_path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                return Err(format!("Directory does not exist: {}", parent.display()));
            }
        }
        
        fs::write(&export_path, ics_content).map_err(|e| e.to_string())?;
        
        Ok(())
    }
    
    fn move_selection(&mut self, dx: i32, dy: i32) {
        let days_delta = dy * 7 + dx;
        if let Some(new_date) = self.selected_date.checked_add_days(chrono::Days::new(days_delta.abs() as u64)) {
            self.selected_date = if days_delta >= 0 { new_date } else { 
                self.selected_date.checked_sub_days(chrono::Days::new(days_delta.abs() as u64)).unwrap()
            };
            
            // Update month view if we've moved to a different month
            if self.selected_date.month() != self.current_month || self.selected_date.year() != self.current_year {
                let local_events = std::mem::take(&mut self.local_events);
                let external_events = std::mem::take(&mut self.external_events);
                let external_events_exportable = std::mem::take(&mut self.external_events_exportable);
                let config = self.config.clone();
                *self = App::with_date(self.selected_date);
                self.local_events = local_events;
                self.external_events = external_events;
                self.external_events_exportable = external_events_exportable;
                self.expand_events();
                self.config = config;
            }
        }
    }
    
    fn change_month(&mut self, delta: i32) {
        let new_month = self.current_month as i32 + delta;
        let (year, month) = if new_month < 1 {
            (self.current_year - 1, 12)
        } else if new_month > 12 {
            (self.current_year + 1, 1)
        } else {
            (self.current_year, new_month as u32)
        };
        
        // Try to keep the same day, or clamp to last day of month
        let first_of_new_month = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
        let new_days_in_month = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap()
        }
        .signed_duration_since(first_of_new_month)
        .num_days() as u32;
        
        let new_day = self.selected_date.day().min(new_days_in_month);
        let new_date = NaiveDate::from_ymd_opt(year, month, new_day).unwrap();
        
        let local_events = std::mem::take(&mut self.local_events);
        let external_events = std::mem::take(&mut self.external_events);
        let external_events_exportable = std::mem::take(&mut self.external_events_exportable);
        let config = self.config.clone();
        *self = App::with_date(new_date);
        self.local_events = local_events;
        self.external_events = external_events;
        self.external_events_exportable = external_events_exportable;
        self.expand_events();
        self.config = config;
    }

    fn events_file() -> PathBuf {
        config_dir().join("events.toml")
    }

    fn load_events(&mut self) {
        // Load local events from file
        let events_file = Self::events_file();
        
        if events_file.exists() {
            if let Ok(content) = fs::read_to_string(&events_file) {
                if let Ok(events_file) = toml::from_str::<EventsFile>(&content) {
                    self.local_events = events_file.events;
                }
            }
        }
        
        // Fetch external ICS events with export flags
        let external_events_with_flags = fetch_ics_events(&self.config);
        self.external_events = external_events_with_flags.iter().map(|(event, _)| event.clone()).collect();
        self.external_events_exportable = external_events_with_flags.iter().map(|(_, exportable)| *exportable).collect();
        
        self.expand_events();
    }
    
    fn expand_events(&mut self) {
        self.events.clear();
        
        // Expand local events
        let local_count = self.local_events.len();
        for event_idx in 0..local_count {
            let event = self.local_events[event_idx].clone();
            self.expand_single_event(&event, EventSource::Local(event_idx));
        }
        
        // Expand external events
        let external_count = self.external_events.len();
        for event_idx in 0..external_count {
            let event = self.external_events[event_idx].clone();
            self.expand_single_event(&event, EventSource::External(event_idx));
        }
        
        // Sort all event lists once after expansion
        let local_events = &self.local_events;
        let external_events = &self.external_events;
        for events in self.events.values_mut() {
            events.sort_by(|a, b| {
                let event_a = match a {
                    EventSource::Local(idx) => &local_events[*idx],
                    EventSource::External(idx) => &external_events[*idx],
                };
                let event_b = match b {
                    EventSource::Local(idx) => &local_events[*idx],
                    EventSource::External(idx) => &external_events[*idx],
                };
                
                // All-day events (no time) come first
                match (event_a.time, event_b.time) {
                    (None, Some(_)) => std::cmp::Ordering::Less,
                    (Some(_), None) => std::cmp::Ordering::Greater,
                    (None, None) => event_a.name.cmp(&event_b.name),
                    (Some(time_a), Some(time_b)) => {
                        time_a.cmp(&time_b).then_with(|| event_a.name.cmp(&event_b.name))
                    }
                }
            });
        }
    }
    
    fn expand_single_event(&mut self, event: &CalendarEvent, source: EventSource) {
        // Add event on start date
        self.events.entry(event.date).or_default().push(source);
        
        // If there's a number, repeat the event that many times (1 = just start date)
        if let Some(number) = event.number {
            if number > 1 {
                let mut current_date = event.date;
                
                for _ in 1..number {
                    // Calculate next occurrence based on repeat interval
                    let next_date = match event.repeat {
                        RepeatInterval::Daily => current_date.succ_opt(),
                        RepeatInterval::Weekly => current_date.checked_add_days(chrono::Days::new(7)),
                        RepeatInterval::Monthly => {
                            // Same day next month
                            let next_month = if current_date.month() == 12 {
                                NaiveDate::from_ymd_opt(current_date.year() + 1, 1, current_date.day())
                            } else {
                                NaiveDate::from_ymd_opt(current_date.year(), current_date.month() + 1, current_date.day())
                            };
                            next_month
                        },
                        RepeatInterval::Yearly => {
                            NaiveDate::from_ymd_opt(current_date.year() + 1, current_date.month(), current_date.day())
                        },
                    };
                    
                    if let Some(next) = next_date {
                        self.events.entry(next).or_default().push(source);
                        current_date = next;
                    } else {
                        break;
                    }
                }
            }
        }
        // If there's an end date, add event on all applicable dates between start and end
        else if let Some(end_date) = event.end_date {
            let mut current_date = event.date;
            
            while current_date < end_date {
                // Calculate next occurrence based on repeat interval
                let next_date = match event.repeat {
                    RepeatInterval::Daily => current_date.succ_opt(),
                    RepeatInterval::Weekly => current_date.checked_add_days(chrono::Days::new(7)),
                    RepeatInterval::Monthly => {
                        // Same day next month
                        let next_month = if current_date.month() == 12 {
                            NaiveDate::from_ymd_opt(current_date.year() + 1, 1, current_date.day())
                        } else {
                            NaiveDate::from_ymd_opt(current_date.year(), current_date.month() + 1, current_date.day())
                        };
                        next_month
                    },
                    RepeatInterval::Yearly => {
                        NaiveDate::from_ymd_opt(current_date.year() + 1, current_date.month(), current_date.day())
                    },
                };
                
                if let Some(next) = next_date {
                    if next <= end_date {
                        self.events.entry(next).or_default().push(source);
                        current_date = next;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
    }

    fn save_event(&mut self, event: CalendarEvent) -> Result<(), String> {
        // Add to local events
        self.local_events.push(event);
        
        // Re-expand all events
        self.expand_events();
        
        // Save local events to file
        self.save_local_events()
    }
    
    fn update_event(&mut self, date: NaiveDate, index: usize, updated_event: CalendarEvent) -> Result<(), String> {
        // Get the sorted list to find the event source
        let sorted_sources = self.get_sorted_event_sources(&date);
        if index >= sorted_sources.len() {
            return Err("Invalid event index".to_string());
        }
        
        // Get the event source
        let source = sorted_sources[index];
        
        // Only allow updating local events
        match source {
            EventSource::Local(idx) => {
                self.local_events[idx] = updated_event;
                
                // Re-expand all events
                self.expand_events();
                
                // Save local events to file
                self.save_local_events()
            }
            EventSource::External(_) => {
                Err("Cannot edit external events".to_string())
            }
        }
    }
    
    fn save_local_events(&self) -> Result<(), String> {
        let events_file = EventsFile { events: self.local_events.clone() };
        let toml_content = toml::to_string_pretty(&events_file).map_err(|e| e.to_string())?;
        fs::write(Self::events_file(), toml_content).map_err(|e| e.to_string())?;
        
        Ok(())
    }
    
    fn get_sorted_event_sources(&self, date: &NaiveDate) -> Vec<EventSource> {
        // Events are already sorted in the HashMap
        self.events
            .get(date)
            .cloned()
            .unwrap_or_default()
    }
    
    fn get_event(&self, source: &EventSource) -> &CalendarEvent {
        match source {
            EventSource::Local(idx) => &self.local_events[*idx],
            EventSource::External(idx) => &self.external_events[*idx],
        }
    }
    
    fn get_sorted_events(&self, date: &NaiveDate) -> Vec<&CalendarEvent> {
        self.get_sorted_event_sources(date)
            .iter()
            .map(|source| self.get_event(source))
            .collect()
    }
}

fn main() -> Result<(), io::Error> {
    let args = Args::parse();
    
    ensure_config_dir()?;
    
    // Parse the target date
    let target_date = if let Some(date_str) = &args.date {
        match NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
            Ok(date) => date,
            Err(_) => {
                eprintln!("Error: Invalid date format. Use YYYY-MM-DD (e.g., 2024-12-26)");
                std::process::exit(1);
            }
        }
    } else {
        Local::now().date_naive()
    };
    
    // If summary mode, print summary and exit
    if args.summary {
        let mut app = App::with_date(target_date);
        app.load_events();
        print_summary(&app, target_date);
        return Ok(());
    }
    
    // If list mode, print upcoming events and exit
    if let Some(count_opt) = args.list {
        let count = count_opt.unwrap_or(5);
        let mut app = App::with_date(target_date);
        app.load_events();
        print_list(&app, target_date, count);
        return Ok(());
    }
    
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::with_date(target_date);
    app.load_events();
    
    let res = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

fn print_summary(app: &App, date: NaiveDate) {
    let config = &app.config;
    
    let events = app.get_sorted_events(&date);
    
    if events.is_empty() {
        println!("No events scheduled for this day.");
    } else {
        for event in events {
            let category_color = event.category.color(config);
            
            // Format time with fixed width for alignment (17 chars total)
            let time_str = if let Some(time) = event.time {
                if let Some(end_time) = event.end_time {
                    format!("{} - {}", time.format("%H:%M"), end_time.format("%H:%M"))
                } else {
                    format!("{}", time.format("%H:%M"))
                }
            } else {
                "All day".to_string()
            };
            
            print_colored_line(&time_str, &event.name, category_color, 17);
        }
    }
}

fn print_list(app: &App, start_date: NaiveDate, count: usize) {
    let config = &app.config;
    
    // Collect all events starting from start_date
    let mut upcoming_events: Vec<(NaiveDate, &CalendarEvent)> = Vec::new();
    
    // Search through the next year of dates to find upcoming events
    let mut current_date = start_date;
    let end_search_date = start_date + chrono::Days::new(365);
    
    while current_date <= end_search_date && upcoming_events.len() < count {
        let events = app.get_sorted_events(&current_date);
        for event in events {
            upcoming_events.push((current_date, event));
            if upcoming_events.len() >= count {
                break;
            }
        }
        current_date = current_date.succ_opt().unwrap_or(current_date);
    }
    
    if upcoming_events.is_empty() {
        println!("No upcoming events found.");
    } else {
        for (date, event) in upcoming_events {
            let category_color = event.category.color(config);
            
            let time_str = if let Some(time) = event.time {
                if let Some(end_time) = event.end_time {
                    format!("{} - {}", time.format("%H:%M"), end_time.format("%H:%M"))
                } else {
                    format!("{}", time.format("%H:%M"))
                }
            } else {
                "All day".to_string()
            };
            
            let date_str = date.format("%a %d %b").to_string();
            let combined = format!("{}    {}", date_str, time_str);
            
            print_colored_line(&combined, &event.name, category_color, 27);
        }
    }
}

fn print_colored_line(prefix: &str, text: &str, color: Color, width: usize) {
    let padded_prefix = format!("{:<width$}", prefix, width = width);
    
    let color_code = match color {
        Color::Black => 30,
        Color::Red => 31,
        Color::Green => 32,
        Color::Yellow => 33,
        Color::Blue => 34,
        Color::Magenta => 35,
        Color::Cyan => 36,
        Color::Gray | Color::DarkGray => 90,
        Color::LightRed => 91,
        Color::LightGreen => 92,
        Color::LightYellow => 93,
        Color::LightBlue => 94,
        Color::LightMagenta => 95,
        Color::LightCyan => 96,
        Color::White => 97,
        Color::Rgb(r, g, b) => {
            println!("\x1b[38;2;{};{};{}m{}    {}\x1b[0m", r, g, b, padded_prefix, text);
            return;
        }
        _ => 37,
    };
    
    println!("\x1b[{}m{}    {}\x1b[0m", color_code, padded_prefix, text);
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            match app.input_mode {
                InputMode::Normal => match key.code {
                    KeyCode::Char('q') => {
                        let _ = app.export_to_ics();
                        return Ok(());
                    }
                    KeyCode::Char('a') => {
                        app.event_form = EventForm::new(app.selected_date);
                        app.input_mode = InputMode::EventPopup;
                    }
                    KeyCode::Char('e') => {
                        app.show_event_numbers = !app.show_event_numbers;
                        app.show_delete_numbers = false;
                    }
                    KeyCode::Char('r') => {
                        app.show_delete_numbers = !app.show_delete_numbers;
                        app.show_event_numbers = false;
                    }
                    KeyCode::Char('i') => {
                        app.input_mode = InputMode::ShowingInfo;
                    }
                    KeyCode::Char('E') => {
                        if let Err(e) = app.export_to_ics() {
                            eprintln!("Failed to export calendar: {}", e);
                        }
                    }
                    KeyCode::Char('/') => {
                        app.search_state = SearchState::new();
                        app.input_mode = InputMode::Searching;
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() && app.show_event_numbers => {
                        let digit = c.to_digit(10).unwrap() as usize;
                        let events = app.get_sorted_events(&app.selected_date);
                        let sorted_sources = app.get_sorted_event_sources(&app.selected_date);
                        if digit > 0 && digit <= events.len() {
                            // Check if it's a local event (editable)
                            match sorted_sources[digit - 1] {
                                EventSource::Local(_) => {
                                    let event_to_edit = events[digit - 1].clone();
                                    app.event_form = EventForm::from_event(&event_to_edit, digit - 1);
                                    app.input_mode = InputMode::EventPopup;
                                }
                                EventSource::External(_) => {
                                    // External events are not editable, do nothing
                                }
                            }
                            app.show_event_numbers = false;
                        }
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() && app.show_delete_numbers => {
                        let digit = c.to_digit(10).unwrap() as usize;
                        let events = app.get_sorted_events(&app.selected_date);
                        let sorted_sources = app.get_sorted_event_sources(&app.selected_date);
                        if digit > 0 && digit <= events.len() {
                            // Check if it's a local event (deletable)
                            match sorted_sources[digit - 1] {
                                EventSource::Local(_) => {
                                    app.input_mode = InputMode::DeletingEvent(digit - 1);
                                }
                                EventSource::External(_) => {
                                    // External events are not deletable, do nothing
                                }
                            }
                            app.show_delete_numbers = false;
                        }
                    }
                    KeyCode::Esc => {
                        app.show_event_numbers = false;
                        app.show_delete_numbers = false;
                    }
                    KeyCode::Left => {
                        app.show_event_numbers = false;
                        app.show_delete_numbers = false;
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            app.change_month(-1);
                        } else {
                            app.move_selection(-1, 0);
                        }
                    }
                    KeyCode::Right => {
                        app.show_event_numbers = false;
                        app.show_delete_numbers = false;
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            app.change_month(1);
                        } else {
                            app.move_selection(1, 0);
                        }
                    }
                    KeyCode::Up => {
                        app.show_event_numbers = false;
                        app.show_delete_numbers = false;
                        app.move_selection(0, -1);
                    }
                    KeyCode::Down => {
                        app.show_event_numbers = false;
                        app.show_delete_numbers = false;
                        app.move_selection(0, 1);
                    }
                    KeyCode::Char(' ') => {
                        app.show_event_numbers = false;
                        app.show_delete_numbers = false;
                        let today = Local::now().date_naive();
                        
                        // Only update the view if we're not already on today
                        if app.selected_date != today {
                            // If moving to different month, update month view
                            if today.month() != app.current_month || today.year() != app.current_year {
                                let local_events = std::mem::take(&mut app.local_events);
                                let external_events = std::mem::take(&mut app.external_events);
                                let external_events_exportable = std::mem::take(&mut app.external_events_exportable);
                                let config = app.config.clone();
                                *app = App::with_date(today);
                                app.local_events = local_events;
                                app.external_events = external_events;
                                app.external_events_exportable = external_events_exportable;
                                app.expand_events();
                                app.config = config;
                            } else {
                                // Same month, just update selected date
                                app.selected_date = today;
                            }
                        }
                    }
                    _ => {}
                },
                InputMode::EventPopup => {
                    // Handle special field selections first - prevent text input to other fields
                    if app.event_form.current_field_idx == 7 {
                        // Category field - only handle 1-9 input, ignore all others
                        match key.code {
                            KeyCode::Char(c) if c.is_ascii_digit() && c >= '1' && c <= '9' => {
                                if let Some(digit) = c.to_digit(10) {
                                    if let Some(category) = EventCategory::from_number(digit) {
                                        app.event_form.category = category;
                                    }
                                }
                                continue;
                            }
                            KeyCode::Char(_) | KeyCode::Backspace => {
                                // Ignore all other character input in category field
                                continue;
                            }
                            _ => {}
                        }
                    } else if app.event_form.current_field_idx == 5 {
                        // Repeat field - only handle D/W/M/Y input, ignore all others
                        match key.code {
                            KeyCode::Char(c) => {
                                match c.to_ascii_lowercase() {
                                    'd' => app.event_form.repeat = RepeatInterval::Daily,
                                    'w' => app.event_form.repeat = RepeatInterval::Weekly,
                                    'm' => app.event_form.repeat = RepeatInterval::Monthly,
                                    'y' => app.event_form.repeat = RepeatInterval::Yearly,
                                    _ => {}
                                }
                                // Ignore all character input in repeat field
                                continue;
                            }
                            KeyCode::Backspace => {
                                // Ignore backspace in repeat field
                                continue;
                            }
                            _ => {}
                        }
                    }
                    
                    match key.code {
                        KeyCode::Tab => app.event_form.next_field(),
                        KeyCode::BackTab => app.event_form.prev_field(),
                        KeyCode::Left => app.event_form.move_field(-1, 0),
                        KeyCode::Right => app.event_form.move_field(1, 0),
                        KeyCode::Up => app.event_form.move_field(0, -1),
                        KeyCode::Down => app.event_form.move_field(0, 1),
                        KeyCode::Enter => {
                            match app.event_form.validate_and_create_event() {
                                Ok(event) => {
                                    if let Some(index) = app.event_form.editing_index {
                                        // Update existing event
                                        if let Err(e) = app.update_event(app.selected_date, index, event) {
                                            eprintln!("Failed to update event: {}", e);
                                        }
                                    } else {
                                        // Create new event
                                        if let Err(e) = app.save_event(event) {
                                            eprintln!("Failed to save event: {}", e);
                                        }
                                    }
                                    app.input_mode = InputMode::Normal;
                                }
                                Err(_) => {
                                    // Validation error - stay in form
                                }
                            }
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Char(c) => {
                            let field = app.event_form.current_field_mut();
                            // Check if the character is valid for this field type at this position
                            if field.can_accept_char(c, field.value.len()) {
                                field.value.push(c);
                            }
                        }
                        KeyCode::Backspace => {
                            app.event_form.current_field_mut().value.pop();
                        }
                        _ => {}
                    }
                },
                InputMode::DeletingEvent(index) => {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            // Confirm deletion
                            let sorted_sources = app.get_sorted_event_sources(&app.selected_date);
                            if index < sorted_sources.len() {
                                // Only allow deleting local events
                                match sorted_sources[index] {
                                    EventSource::Local(event_idx) => {
                                        // Remove from local_events
                                        app.local_events.remove(event_idx);
                                        // Re-expand all events
                                        app.expand_events();
                                        if let Err(e) = app.save_local_events() {
                                            eprintln!("Failed to save events: {}", e);
                                        }
                                    }
                                    EventSource::External(_) => {
                                        // Cannot delete external events
                                    }
                                }
                            }
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            // Cancel deletion
                            app.input_mode = InputMode::Normal;
                        }
                        _ => {}
                    }
                }
                InputMode::ShowingInfo => {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('i') | KeyCode::Char('q') => {
                            app.input_mode = InputMode::Normal;
                        }
                        _ => {}
                    }
                }
                InputMode::Searching => {
                    match key.code {
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Tab => {
                            app.search_state.cycle_mode();
                            app.search_state.update_results(&app.local_events, &app.external_events, &app.config);
                        }
                        KeyCode::Enter => {
                            if let Some(date) = app.search_state.get_selected_date() {
                                let local_events = std::mem::take(&mut app.local_events);
                                let external_events = std::mem::take(&mut app.external_events);
                                let external_events_exportable = std::mem::take(&mut app.external_events_exportable);
                                let config = app.config.clone();
                                *app = App::with_date(date);
                                app.local_events = local_events;
                                app.external_events = external_events;
                                app.external_events_exportable = external_events_exportable;
                                app.expand_events();
                                app.config = config;
                                app.input_mode = InputMode::Normal;
                            }
                        }
                        KeyCode::Up => {
                            app.search_state.move_selection(-1);
                        }
                        KeyCode::Down => {
                            app.search_state.move_selection(1);
                        }
                        KeyCode::Char(c) => {
                            app.search_state.query.push(c);
                            app.search_state.update_results(&app.local_events, &app.external_events, &app.config);
                        }
                        KeyCode::Backspace => {
                            app.search_state.query.pop();
                            app.search_state.update_results(&app.local_events, &app.external_events, &app.config);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn ui(f: &mut ratatui::Frame, app: &App) {
    let size = f.area();
    
    // Split main area into two columns (no footer)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(size);
    
    // Left column: Calendar
    render_calendar(f, app, chunks[0]);
    
    // Right column: Selected date info
    render_date_info(f, app, chunks[1]);
    
    // Render popup if in input mode
    if app.input_mode == InputMode::EventPopup {
        render_event_popup(f, app);
    } else if let InputMode::DeletingEvent(index) = app.input_mode {
        render_delete_confirmation(f, app, index);
    } else if app.input_mode == InputMode::ShowingInfo {
        render_info_popup(f, app);
    } else if app.input_mode == InputMode::Searching {
        render_search_popup(f, app);
    }
}

fn render_calendar(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    use ratatui::text::{Line, Span};
    
    let month_names = [
        "January", "February", "March", "April", "May", "June",
        "July", "August", "September", "October", "November", "December"
    ];
    let title = format!("{} {}", month_names[(app.current_month - 1) as usize], app.current_year);
    
    let header_cells = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::DarkGray)));
    let header = Row::new(header_cells).height(1);
    
    let mut rows = vec![];
    let total_cells = ((app.first_weekday + app.days_in_month + 6) / 7) * 7;
    let num_weeks = total_cells / 7;
    
    // Calculate first date to display (might be from previous month)
    let first_of_month = NaiveDate::from_ymd_opt(app.current_year, app.current_month, 1).unwrap();
    let first_display_date = first_of_month - chrono::Days::new(app.first_weekday as u64);
    
    // Calculate row height to fill the area
    // Area height minus borders (2), padding (2), and header (1)
    let available_height = area.height.saturating_sub(5);
    let base_row_height = (available_height / num_weeks as u16).max(3);
    let extra_space = available_height - (base_row_height * num_weeks as u16);
    
    let mut week_idx = 0;
    for week_start in (0..total_cells).step_by(7) {
        // Distribute extra space to first rows
        let row_height = if (week_idx as u16) < extra_space {
            base_row_height + 1
        } else {
            base_row_height
        };
        
        let cells: Vec<Cell> = (0..7)
            .map(|day_offset| {
                let pos = week_start + day_offset;
                let cell_date = first_display_date + chrono::Days::new(pos as u64);
                
                let is_current_month = cell_date.month() == app.current_month;
                let is_selected = cell_date == app.selected_date;
                let is_today = cell_date == Local::now().date_naive();
                
                // Get events for this date
                let events = app.events.get(&cell_date);
                
                // Build cell content with day number and event indicators
                let mut lines = vec![];
                
                // First line: day number
                let day_style = if is_selected {
                    Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD)
                } else if is_today {
                    Style::default().add_modifier(Modifier::BOLD)
                } else if !is_current_month {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                };
                
                if is_today {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{}", cell_date.day()), day_style),
                        Span::styled(" 󰃮", Style::default().fg(Color::White)),
                    ]));
                } else {
                    lines.push(Line::from(Span::styled(format!("{}", cell_date.day()), day_style)));
                }
                
                // Add event indicator lines (colored bars) - sorted like day view
                if let Some(event_sources) = events {
                    // Get sorted events using the event sources
                    let sorted_events: Vec<&CalendarEvent> = event_sources.iter()
                        .map(|source| app.get_event(source))
                        .collect();
                    
                    // Separate all-day and timed events
                    let all_day_events: Vec<_> = sorted_events.iter().filter(|e| e.time.is_none()).copied().collect();
                    let timed_events: Vec<_> = sorted_events.iter().filter(|e| e.time.is_some()).copied().collect();
                    
                    let max_indicators = (row_height as usize).saturating_sub(1);
                    let mut lines_used = 0;
                    
                    // Process all-day events in pairs (compress 2 events per line)
                    let mut i = 0;
                    while i < all_day_events.len() && lines_used < max_indicators {
                        if i + 1 < all_day_events.len() {
                            // Two all-day events: stack them (upper=first, lower=second)
                            let event1 = all_day_events[i];
                            let event2 = all_day_events[i + 1];
                            let bar = "▀".repeat(8);
                            let bar_style = Style::default()
                                .fg(event1.category.color(&app.config))
                                .bg(event2.category.color(&app.config));
                            lines.push(Line::from(Span::styled(bar, bar_style)));
                            i += 2;
                        } else {
                            // Single all-day event (last one if odd number)
                            let event = all_day_events[i];
                            let bar = "▀".repeat(8);
                            let bar_style = Style::default().fg(event.category.color(&app.config));
                            lines.push(Line::from(Span::styled(bar, bar_style)));
                            i += 1;
                        }
                        lines_used += 1;
                    }
                    
                    // Process timed events (one per line)
                    for event in timed_events.iter().take(max_indicators.saturating_sub(lines_used)) {
                        if let Some(start_time) = event.time {
                            let time_str = start_time.format("%H:%M").to_string();
                            let time_style = Style::default().fg(event.category.color(&app.config));
                            lines.push(Line::from(vec![
                                Span::styled(time_str, time_style),
                            ]));
                        }
                    }
                }
                
                // Pad with empty lines to fill row height
                while lines.len() < row_height as usize {
                    let empty_style = if is_selected {
                        Style::default().bg(Color::DarkGray)
                    } else {
                        Style::default()
                    };
                    lines.push(Line::from(Span::styled(" ", empty_style)));
                }
                
                // Create cell with background color if selected
                let mut cell = Cell::from(lines);
                if is_selected {
                    cell = cell.style(Style::default().bg(Color::DarkGray));
                }
                cell
            })
            .collect();
        rows.push(Row::new(cells).height(row_height));
        week_idx += 1;
    }
    
    let widths = [
        Constraint::Percentage(14),
        Constraint::Percentage(14),
        Constraint::Percentage(14),
        Constraint::Percentage(15),
        Constraint::Percentage(14),
        Constraint::Percentage(14),
        Constraint::Percentage(15),
    ];
    
    // Create block with no bottom padding so keybinds touch the border
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title))
        .padding(ratatui::widgets::Padding {
            left: 1,
            right: 0,
            top: 1,
            bottom: 0,
        });
    
    let inner = block.inner(area);
    f.render_widget(block, area);
    
    // Split inner area to add keybinds at bottom inside the border
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    
    let table = Table::new(rows, widths)
        .header(header);
    
    f.render_widget(table, chunks[0]);
    
    // Render keybinds at bottom inside the border
    let keybinds = Paragraph::new("/: search | E: export | Space: today | i: info | q: quit")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(keybinds, chunks[1]);
}

fn render_date_info(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    let title = format!(" {} ", app.selected_date.format("%A %d").to_string());
    
    // Create block with no bottom padding so keybinds touch the border
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title.clone())
        .padding(ratatui::widgets::Padding {
            left: 1,
            right: 1,
            top: 1,
            bottom: 0,
        });
    
    let inner = block.inner(area);
    f.render_widget(block, area);
    
    // Split inner area to add keybinds at bottom inside the border
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    
    let events_list = app.get_sorted_events(&app.selected_date);
    let sources = app.get_sorted_event_sources(&app.selected_date);
    
    if events_list.is_empty() {
        // No events - show empty message
        let info = Paragraph::new("No events")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(info, chunks[0]);
    } else {
        // Build table rows with spacing between all-day and timed events
        let mut rows: Vec<Row> = Vec::new();
        let mut last_was_all_day = false;
        
        for (idx, event) in events_list.iter().enumerate() {
            let is_all_day = event.time.is_none();
            
            // Add spacing row after all-day events section
            if idx > 0 && last_was_all_day && !is_all_day {
                rows.push(Row::new(vec![
                    Cell::from(""),
                    Cell::from(""),
                    Cell::from(""),
                ]).height(1));
            }
            
            // Number column (bright purple when visible) or external indicator
            let source = &sources[idx];
            let number_cell = match source {
                EventSource::External(_) => {
                    // Show external indicator (nerd font calendar icon)
                    Cell::from("󰃭").style(Style::default().fg(Color::DarkGray))
                }
                EventSource::Local(_) => {
                    if app.show_event_numbers || app.show_delete_numbers {
                        Cell::from(format!("{}", idx + 1)).style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
                    } else {
                        Cell::from("")
                    }
                }
            };
            
            let time_str = if let Some(start) = event.time {
                if let Some(end) = event.end_time {
                    format!("{} - {}", start.format("%H:%M"), end.format("%H:%M"))
                } else {
                    start.format("%H:%M").to_string()
                }
            } else {
                String::new()
            };
            
            rows.push(Row::new(vec![
                number_cell,
                Cell::from(time_str).style(Style::default().fg(Color::White)),
                Cell::from(event.name.as_str()).style(Style::default().fg(event.category.color(&app.config))),
            ]));
            
            last_was_all_day = is_all_day;
        }
        
        let widths = [
            Constraint::Length(3),   // Number column
            Constraint::Length(15),  // Time column
            Constraint::Min(20),     // Event name column
        ];
        
        let table = Table::new(rows, widths)
            .style(Style::default().fg(Color::White));
        
        f.render_widget(table, chunks[0]);
    }
    
    // Render keybinds at bottom inside the border
    let keybinds = Paragraph::new("a: add | e: edit | r: remove")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(keybinds, chunks[1]);
}

fn render_event_popup(f: &mut ratatui::Frame, app: &App) {
    let area = f.area();
    
    let is_editing = app.event_form.editing_index.is_some();
    
    // Create centered popup
    let popup_width = 80.min(area.width - 4);
    let popup_height = 18;  // Increased height for repeat field
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    
    let popup_area = ratatui::layout::Rect {
        x: popup_x,
        y: popup_y,
        width: popup_width,
        height: popup_height,
    };
    
    // Clear the area behind the popup to prevent bleed-through
    f.render_widget(ratatui::widgets::Clear, popup_area);
    
    // Create popup border with solid background - add error message to title if present
    let title = if let Some(error) = app.event_form.get_first_error() {
        format!(" {} - {} ", if is_editing { "Edit Event" } else { "Add Event" }, error)
    } else if is_editing {
        " Edit Event ".to_string()
    } else {
        " Add Event ".to_string()
    };
    
    let title_style = if app.event_form.get_first_error().is_some() {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::White)
    };
    
    let block = Block::default()
        .borders(Borders::ALL)
        .title(ratatui::text::Span::styled(title, title_style))
        .style(Style::default().bg(Color::Reset).fg(Color::White))
        .padding(ratatui::widgets::Padding {
            left: 1,
            right: 1,
            top: 1,
            bottom: 0,
        });
    
    f.render_widget(block.clone(), popup_area);
    
    // Inner area for content
    let inner = block.inner(popup_area);
    
    // Split into rows: Name (full width), Date row (2 cols), Time row (2 cols), Space, Repeat+Number row, Category field, Help text
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Name field
            Constraint::Length(1), // Space
            Constraint::Length(1), // Date fields
            Constraint::Length(1), // Space
            Constraint::Length(1), // Time fields
            Constraint::Length(1), // Space
            Constraint::Length(1), // Repeat + Number fields
            Constraint::Length(1), // Space
            Constraint::Length(4), // Category field with table
            Constraint::Min(1),    // Space
            Constraint::Length(1), // Help text
        ])
        .split(inner);
    
    // Render Name field (spans full width)
    let has_error_0 = app.event_form.get_field_error(0).is_some();
    render_field(f, &app.event_form.fields[0], layout[0], app.event_form.current_field_idx == 0, has_error_0);
    
    // Split date row into two columns
    let date_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(layout[2]);
    
    let has_error_1 = app.event_form.get_field_error(1).is_some();
    let has_error_2 = app.event_form.get_field_error(2).is_some();
    render_field(f, &app.event_form.fields[1], date_cols[0], app.event_form.current_field_idx == 1, has_error_1);
    render_field(f, &app.event_form.fields[2], date_cols[1], app.event_form.current_field_idx == 2, has_error_2);
    
    // Split time row into two columns
    let time_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(layout[4]);
    
    let has_error_3 = app.event_form.get_field_error(3).is_some();
    let has_error_4 = app.event_form.get_field_error(4).is_some();
    render_field(f, &app.event_form.fields[3], time_cols[0], app.event_form.current_field_idx == 3, has_error_3);
    render_field(f, &app.event_form.fields[4], time_cols[1], app.event_form.current_field_idx == 4, has_error_4);
    
    // Split repeat row into two columns
    let repeat_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(layout[6]);
    
    // Render repeat selector as a field
    render_repeat_field(f, app, repeat_cols[0]);
    
    // Render number field
    let has_error_6 = app.event_form.get_field_error(6).is_some();
    render_field(f, &app.event_form.fields[6], repeat_cols[1], app.event_form.current_field_idx == 6, has_error_6);
    
    // Render category selector as a field
    render_category_field(f, app, layout[8]);
    
    // Help text
    let help = Paragraph::new("Tab/Arrows: navigate | Enter: save | Esc: cancel")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(help, layout[10]);
    
    // Position cursor
    let current_field = &app.event_form.fields[app.event_form.current_field_idx];
    let field_area = match app.event_form.current_field_idx {
        0 => layout[0],
        1 => date_cols[0],
        2 => date_cols[1],
        3 => time_cols[0],
        4 => time_cols[1],
        5 => repeat_cols[0], // Repeat field
        6 => repeat_cols[1], // Number field
        7 => layout[8], // Category field
        _ => layout[0],
    };
    
    // Calculate label length: "Label: " (no asterisk)
    let label_str = format!("{}: ", current_field.label);
    let cursor_x = field_area.x + label_str.len() as u16 + current_field.value.len() as u16;
    let cursor_y = field_area.y;
    
    f.set_cursor_position((cursor_x, cursor_y));
}

fn render_category_field(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    use ratatui::text::{Line, Span};
    
    let is_selected = app.event_form.current_field_idx == 7;
    let categories = EventCategory::all();
    
    // Build label line
    let label_style = if is_selected {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::Gray)
    };
    
    let selected_cat = app.event_form.category;
    let value_style = if is_selected {
        selected_cat.color(&app.config)
    } else {
        selected_cat.color(&app.config)
    };
    
    let label_line = Line::from(vec![
        Span::styled("Category: ", label_style),
        Span::styled(selected_cat.name(&app.config), Style::default().fg(value_style)),
        if is_selected {
            Span::styled(" (1-9)", Style::default().fg(Color::DarkGray))
        } else {
            Span::raw("")
        },
    ]);
    
    let label_para = Paragraph::new(label_line);
    
    // Calculate area for label (first line)
    let label_area = ratatui::layout::Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    
    f.render_widget(label_para, label_area);
    
    // Only show category table when field is selected
    if is_selected {
        let table_area = ratatui::layout::Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(1),
        };
        
        // Build 3x3 table rows
        let mut rows = Vec::new();
        
        for row_idx in 0..3 {
            let cells: Vec<Cell> = (0..3)
                .map(|col_idx| {
                    let cat_idx = row_idx * 3 + col_idx;
                    let category = categories[cat_idx];
                    let num = category as u32;
                    let name = category.name(&app.config);
                    
                    let style = Style::default().fg(category.color(&app.config));
                    
                    Cell::from(format!("{}: {}", num, name)).style(style)
                })
                .collect();
            
            rows.push(Row::new(cells).height(1));
        }
        
        let widths = [
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ];
        
        let table = Table::new(rows, widths)
            .style(Style::default());
        
        f.render_widget(table, table_area);
    }
}

fn render_repeat_field(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    use ratatui::text::{Line, Span};
    
    let is_selected = app.event_form.current_field_idx == 5;
    
    // Build label line
    let label_style = if is_selected {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::Gray)
    };
    
    let repeat_value = app.event_form.repeat.to_string();
    let value_style = if is_selected {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::Cyan)
    };
    
    let label_line = Line::from(vec![
        Span::styled("Repeat: ", label_style),
        Span::styled(&repeat_value, value_style),
        if is_selected {
            Span::styled(" (d,w,m,y)", Style::default().fg(Color::DarkGray))
        } else {
            Span::raw("")
        },
    ]);
    
    let label_para = Paragraph::new(label_line);
    f.render_widget(label_para, area);
}

fn render_field(f: &mut ratatui::Frame, field: &FormField, area: ratatui::layout::Rect, is_selected: bool, has_error: bool) {
    use ratatui::text::{Line, Span};
    
    let label = format!("{}: ", field.label);
    
    let display_value = if field.value.is_empty() {
        field.placeholder.as_str()
    } else {
        field.value.as_str()
    };
    
    let label_style = if has_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::White)
    };
    
    let value_style = if is_selected {
        Style::default().bg(Color::Blue).fg(Color::White)
    } else if field.value.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };
    
    let line = Line::from(vec![
        Span::styled(label, label_style),
        Span::styled(display_value, value_style),
    ]);
    
    let paragraph = Paragraph::new(line);
    f.render_widget(paragraph, area);
}

fn render_delete_confirmation(f: &mut ratatui::Frame, app: &App, event_index: usize) {
    use ratatui::text::{Line, Span};
    
    let events_list = app.get_sorted_events(&app.selected_date);
    if event_index >= events_list.len() {
        return;
    }
    
    let event = events_list[event_index];
    
    // Calculate popup size and position (centered)
    let area = f.area();
    let popup_width = 60.min(area.width - 4);
    let popup_height = 8;
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    
    let popup_area = ratatui::layout::Rect {
        x: popup_x,
        y: popup_y,
        width: popup_width,
        height: popup_height,
    };
    
    // Clear the popup area with background
    f.render_widget(ratatui::widgets::Clear, popup_area);
    
    // Build confirmation message
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("Delete this event?", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled(format!("  {}", event.name), Style::default().fg(Color::White))),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::White)),
            Span::styled("Y", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(" to confirm or ", Style::default().fg(Color::White)),
            Span::styled("N/ESC", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(" to cancel", Style::default().fg(Color::White)),
        ]),
    ];
    
    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Confirm Deletion "))
        .style(Style::default().bg(Color::Reset));
    
    f.render_widget(paragraph, popup_area);
}

fn render_info_popup(f: &mut ratatui::Frame, app: &App) {
    use ratatui::text::{Line, Span};
    
    let area = f.area();
    
    // Calculate popup size (centered)
    let popup_width = 60.min(area.width - 4);
    let popup_height = 15.min(area.height - 4);
    let popup_x = (area.width - popup_width) / 2;
    let popup_y = (area.height - popup_height) / 2;
    
    let popup_area = ratatui::layout::Rect {
        x: popup_x,
        y: popup_y,
        width: popup_width,
        height: popup_height,
    };
    
    // Get paths
    let config_dir_path = config_dir();
    let dir_display = config_dir_path.display().to_string();
    
    // Build export path display
    let export_path_display = if let Some(path) = &app.config.export_path {
        path.to_string()
    } else {
        format!("{}/calendar.ics", dir_display)
    };
    
    // Clear the area behind the popup to prevent bleed-through
    f.render_widget(ratatui::widgets::Clear, popup_area);
    
    // Build info text
    let lines = vec![
        Line::from(vec![
            Span::styled("Version: ", Style::default().fg(Color::Gray)),
            Span::raw("calcite 0.2.0-beta"),
        ]),
        Line::from(vec![
            Span::styled("Local Events: ", Style::default().fg(Color::Gray)),
            Span::raw(format!("{}", app.local_events.len())),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("ICS Calendars: ", Style::default().fg(Color::Gray)),
            Span::raw(format!("{}", app.config.ics_calendars.len())),
        ]),
        Line::from(vec![
            Span::styled("External Events: ", Style::default().fg(Color::Gray)),
            Span::raw(format!("{}", app.external_events.len())),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Local Directory: ", Style::default().fg(Color::Gray)),
            Span::raw(dir_display),
        ]),
        Line::from(vec![
            Span::styled("Export Path: ", Style::default().fg(Color::Gray)),
            Span::raw(export_path_display),
        ]),
    ];
    
    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Info (ESC to close) ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .style(Style::default().bg(Color::Reset).fg(Color::White))
                .padding(ratatui::widgets::Padding {
                    left: 1,
                    right: 1,
                    top: 1,
                    bottom: 0,
                })
        );
    
    f.render_widget(paragraph, popup_area);
}

fn render_search_popup(f: &mut ratatui::Frame, app: &App) {
    use ratatui::text::{Line, Span};
    
    let area = f.area();
    
    // Calculate popup size (centered)
    let popup_width = 70.min(area.width - 4);
    let popup_height = 20.min(area.height - 4);
    let popup_x = (area.width - popup_width) / 2;
    let popup_y = (area.height - popup_height) / 2;
    
    let popup_area = ratatui::layout::Rect {
        x: popup_x,
        y: popup_y,
        width: popup_width,
        height: popup_height,
    };
    
    // Clear the area behind the popup to prevent bleed-through
    f.render_widget(ratatui::widgets::Clear, popup_area);
    
    // Create popup border with padding only on top, left, and right (not bottom)
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Search Events ")
        .style(Style::default().bg(Color::Reset).fg(Color::White))
        .padding(ratatui::widgets::Padding {
            left: 1,
            right: 1,
            top: 1,
            bottom: 0,
        });
    
    f.render_widget(block.clone(), popup_area);
    
    let inner = block.inner(popup_area);
    
    // Split into search input and results
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Search input
            Constraint::Min(0),    // Results list
            Constraint::Length(1), // Empty line before help
            Constraint::Length(1), // Help text
        ])
        .split(inner);
    
    // Render search input with mode indicator
    let query_display = if app.search_state.query.is_empty() {
        "Type to search..."
    } else {
        &app.search_state.query
    };
    
    let query_style = if app.search_state.query.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White).bg(Color::Blue)
    };
    
    let mode_str = app.search_state.mode.to_string();
    let mode_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    
    let query_line = Line::from(vec![
        Span::styled("Search [", Style::default().fg(Color::White)),
        Span::styled(mode_str, mode_style),
        Span::styled("]: ", Style::default().fg(Color::White)),
        Span::styled(query_display, query_style),
    ]);
    
    let query_para = Paragraph::new(query_line);
    f.render_widget(query_para, layout[0]);
    
    // Render results
    if app.search_state.results.is_empty() {
        if !app.search_state.query.is_empty() {
            let no_results = Paragraph::new("No results found")
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(no_results, layout[1]);
        }
    } else {
        let mut rows = Vec::new();
        
        for (idx, result) in app.search_state.results.iter().enumerate() {
            let is_selected = idx == app.search_state.selected_index;
            
            let event = app.get_event(&result.source);
            let date_str = result.date.format("%Y-%m-%d").to_string();
            
            let time_str = if let Some(time) = result.time {
                time.format("%H:%M").to_string()
            } else {
                "All day".to_string()
            };
            
            let style = if is_selected {
                Style::default().bg(Color::Blue).fg(Color::White)
            } else {
                Style::default().fg(Color::White)
            };
            
            let event_color = event.category.color(&app.config);
            
            rows.push(Row::new(vec![
                Cell::from(date_str).style(style),
                Cell::from(time_str).style(style),
                Cell::from(result.event_name.as_str()).style(if is_selected {
                    Style::default().bg(Color::Blue).fg(event_color)
                } else {
                    Style::default().fg(event_color)
                }),
            ]).height(1));
        }
        
        let widths = [
            Constraint::Length(12),  // Date
            Constraint::Length(10),  // Time
            Constraint::Min(20),     // Event name
        ];
        
        let table = Table::new(rows, widths)
            .style(Style::default());
        
        f.render_widget(table, layout[1]);
    }
    
    // Help text (layout[2] is the empty line, layout[3] is the help text)
    let help = Paragraph::new("↑↓: navigate | Tab: cycle mode | Enter: select | Esc: cancel")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(help, layout[3]);
    
    // Position cursor after search input - adjust for new prompt format
    // "Search [" = 8, mode string, "]: " = 3, then query position
    let mode_str = app.search_state.mode.to_string();
    let cursor_x = layout[0].x + 8 + mode_str.len() as u16 + 3 + app.search_state.query.len() as u16;
    let cursor_y = layout[0].y;
    f.set_cursor_position((cursor_x, cursor_y));
}



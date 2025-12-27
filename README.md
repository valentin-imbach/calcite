# calcite - Terminal Calendar

A feature-rich TUI calendar application built with Rust and Ratatui.

> ⚠️ **Note**: This project is in active development. Features and functionality may change.

## Features

- **Month View**: 7-column calendar grid (Monday to Sunday) with visual event indicators
  - Colored bars show events on each day
  - Upper/lower half-blocks for efficient all-day event display (2 events per line)
  - Time indicators (bullets with times) for timed events
  - Navigate between months with Shift+Left/Right
  
- **Day View**: Detailed event list for selected date
  - Sorted display: all-day events first, then timed events by start time
  - Color-coded by category
  - Quick edit/delete with keyboard hints

- **Event Management**:
  - Add, edit, and delete events with full validation
  - **Fields**: Name, Date, End Date, Time, End Time, Repeat, Number, Category
  - **Repeat Options**: Daily, Weekly, Monthly, Yearly
  - **Number**: Repeat event N times (e.g., "3" = event occurs 3 times)
  - End Date OR Number (mutually exclusive for defining event duration)
  - Smart input validation (date/time format checking, leap years, valid date ranges)
  - 9 customizable color-coded categories

- **External Calendars**: Import ICS calendars from URLs (read-only)

- **Storage**: Local events stored in `events.toml`, auto-saved on changes

## Installation

### From Source

```bash
# Clone the repository
git clone <repository-url>
cd calcite

# Install with cargo
cargo install --path .

# Or run directly
cargo run --release
```

### Via Cargo (when published)

```bash
cargo install calcite
```

## Configuration

Configuration files are stored in `~/.config/calcite/`:

- **config.toml**: Categories, external calendars, export settings
- **events.toml**: Local events storage (auto-saved)

See the example `config.toml` in the repository for all available options.

## Usage

### TUI Mode (default)

```bash
calcite                      # Open calendar with today's date
calcite --date 2025-01-15    # Open with specific date selected
calcite -d 2025-01-15        # Short form
```

### Command-Line Mode

```bash
# Show events for today
calcite --summary
calcite -s

# Show events for a specific date
calcite --summary --date 2025-01-15
calcite -s -d 2025-01-15

# List next 5 upcoming events (default)
calcite --list
calcite -l

# List next 10 upcoming events
calcite --list 10
calcite -l 10

# List upcoming events from a specific date
calcite -l 5 -d 2025-01-15
```

## Controls

### Navigation
- **Arrow Keys**: Navigate between days (including previous/next month cells)
- **Shift + Left/Right**: Change month
- **Space**: Jump to today's date

### Actions
- **a**: Add event to selected date
- **e**: Show event numbers for editing
- **r**: Show event numbers for removal
- **i**: Show info/debug popup
- **q**: Quit application

### Event Management
- **1-9** (when hints visible): Select event to edit/remove
- **ESC**: Cancel current action/close popup

### In Event Popup
- **Arrow Keys / Tab**: Navigate between fields
- **Type**: Enter values in text fields
- **d, w, m, y**: Set repeat mode (Daily, Weekly, Monthly, Yearly) when Repeat field selected
- **1-9**: Select category (when Category field selected)
- **Enter**: Save event (validates all fields)
- **ESC**: Cancel and close

### Event Fields
- **Name**: Event title (required)
- **Date**: Start date in YYYY-MM-DD format (required, defaults to selected date)
- **End Date**: Optional end date for multi-day events (mutually exclusive with Number)
- **Time**: Optional start time in HH:MM format (24-hour, omit for all-day events)
- **End Time**: Optional end time (requires Time to be set)
- **Repeat**: Daily/Weekly/Monthly/Yearly (default: Daily)
- **Number**: Repeat N times (mutually exclusive with End Date)
- **Category**: 1-9, color-coded (default: 1)

## Dependencies

- `ratatui`: Terminal UI framework
- `crossterm`: Cross-platform terminal handling
- `chrono`: Date and time library (handles leap years, month lengths, etc.)
- `serde`: Serialization framework
- `toml`: TOML file parsing
- `ical`: ICS calendar parsing
- `reqwest`: HTTP client for fetching external calendars
- `clap`: Command-line argument parser
- `dirs`: System directory paths
- `fuzzy-matcher`: Fuzzy search functionality

## Roadmap

- [ ] Week/year views
- [ ] Event search and filtering
- [ ] Recurring event exceptions
- [x] Export to ICS format
- [x] Command-line interface for quick queries
- [ ] Custom keybindings

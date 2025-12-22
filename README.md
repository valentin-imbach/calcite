# vCal - Terminal Calendar

A simple TUI calendar app built with Rust and Ratatui.

## Configuration

Categories are configurable via `config.toml`. If the file doesn't exist, default categories will be used. See `config.example.toml` for reference.

Available colors: Black, Red, Green, Yellow, Blue, Magenta, Cyan, Gray, DarkGray, LightRed, LightGreen, LightYellow, LightBlue, LightMagenta, LightCyan, White, Orange, Brown, Teal

## Features

- Displays current month in a 7-column grid (Monday to Sunday)
- Navigate cells using arrow keys
- Current date is pre-selected
- Add events with name, dates, and times
- Events stored in TOML format in `./events.toml`
- View events for selected date in right panel
- Press `q` to quit

## Usage

```bash
cargo run
```

## Controls

- **Arrow Keys**: Navigate between days (including previous/next month days)
- **Shift + Left/Right**: Change month
- **Space**: Jump to today's date
- **a**: Add event to selected date (opens popup)
- **e**: Toggle event numbers (for editing)
- **1-9**: Edit event by number (when numbers visible)
- **q**: Quit the application

### In Event Popup:
- **Arrow Keys**: Navigate between fields (2-column grid layout)
- **Tab/Shift+Tab**: Navigate through fields sequentially
- **Type**: Enter field values
  - Name* (required)
  - Start Date* (YYYY-MM-DD, defaults to selected date)
  - End Date (YYYY-MM-DD, optional, for multi-day events)
  - Start Time (HH:MM, optional, omit for all-day events)
  - End Time (HH:MM, optional)
- **Enter**: Save event (with validation)
- **Esc**: Cancel and close popup

## Dependencies

- `ratatui`: Terminal UI framework
- `crossterm`: Cross-platform terminal handling
- `chrono`: Date and time library (handles leap years, month lengths, etc.)

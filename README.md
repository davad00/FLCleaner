# FL Studio Backup Cleaner

A utility tool that helps FL Studio users manage their project backup files by scanning drives and cleaning up old backups while keeping the latest version of each project.

![FL Studio Backup Cleaner Screenshot](images/screenshot-1.jpg)

## Features

- **Intelligent Scanning**: Quickly scans your drives for FL Studio backup files with multithreaded performance
- **Smart Cleanup**: Automatically keeps the latest backup of each project while removing older versions
- **Customizable Settings**: Configure scan depth, thread count, and drive selection to match your system
- **Light & Dark Themes**: Choose between light and dark modes for comfortable use in any environment
- **Real-time Progress**: Monitor scan progress with detailed information and color-coded indicators
- **Safe Operation**: Preview what will be deleted before cleaning to ensure you never lose important files

## Download

Download the latest version from the [Releases](https://github.com/davad00/FLCleaner/releases) page.

### Options:
- [Installer Version](https://github.com/davad00/FLCleaner/releases/download/v0.1/FruityCleaner_Installer_1.0.0.exe)
- [Portable Version (ZIP)](https://github.com/davad00/FLCleaner/releases/download/v0.1/FruityCleaner_StandAlone_1.0.0.zip)

## How It Works

1. **Scan Your Drives**: Select which drives to scan and start the process with a single click
2. **Review Findings**: See all discovered FL Studio backup files organized by project
3. **Clean Up**: Click "Clean Old Backups" to remove outdated files while keeping the latest version of each project
4. **Enjoy Free Space**: See how much disk space you've reclaimed and enjoy your tidier system

## Building from Source

### Prerequisites

- Rust 1.70.0 or newer
- Cargo package manager

### Build Steps

1. Clone the repository:
   ```
   git clone https://github.com/davad00/FLCleaner.git
   cd FLCleaner
   ```

2. Build the project:
   ```
   cargo build --release
   ```

3. The compiled executable will be in `target/release/flcleaner.exe`

## Website

This repository also contains the website for the FL Studio Backup Cleaner application, which is hosted via GitHub Pages.

### Website Development

The website files are located in the root directory:

- `index.html` - Main HTML file
- `styles.css` - CSS styles for the website
- `script.js` - JavaScript functionality
- `images/` - Directory containing all website images

To work on the website locally, simply open `index.html` in your browser.

### Deployment

The website is automatically deployed from the `gh-pages` branch. See [GitHub Pages Deployment Guide](github_pages_deployment.md) for more details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Disclaimer

FL Studio is a registered trademark of Image-Line Software. This tool is not affiliated with or endorsed by Image-Line Software.

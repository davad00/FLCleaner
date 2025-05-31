// Cargo.toml dependencies needed:
// [dependencies]
// eframe = { version = "0.24", features = ["default"] }
// walkdir = "2.3"
// regex = "1.7"
// chrono = { version = "0.4", features = ["serde"] }
// tokio = { version = "1.0", features = ["full"] }
// serde = { version = "1.0", features = ["derive"] }
// serde_json = "1.0"
// dirs = "5.0"
// 
// [target.'cfg(windows)'.dependencies]
// winapi = { version = "0.3", features = ["winuser", "windef", "shellapi", "objbase", "combaseapi"] }

// Set Windows subsystem to hide console window
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use walkdir::WalkDir;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

#[derive(Default)]
struct FlBackupCleaner {
    scan_status: String,
    found_backups: HashMap<String, Vec<BackupFile>>, // Key: full project path, Value: backup files
    total_files_found: usize,
    total_size_saved: u64,
    is_scanning: bool,
    scan_complete: bool,
    deletion_status: String,
    scan_receiver: Option<mpsc::Receiver<ScanMessage>>,
    scan_progress: String,
    progress_percentage: f32,
    scan_start_time: Option<Instant>,
    files_scanned: usize,
    total_files_estimated: usize,
    theme: Theme,
    error_messages: Vec<String>,  // Added to track error messages
    detailed_progress: Vec<String>,  // Added to track detailed progress
    settings_open: bool,  // Track if settings modal is open
    available_drives: Vec<DriveInfo>,  // List of available drives
    settings: Settings,  // User settings
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Theme {
    Light,
    Dark,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::Dark
    }
}

// Settings structure to store user preferences
#[derive(Clone, Serialize, Deserialize)]
struct Settings {
    theme: Theme,
    selected_drives: HashSet<String>,  // Drives selected for scanning
    max_scan_threads: usize,  // Maximum number of scanning threads
    scan_depth: usize,  // Maximum directory depth to scan
    auto_clean: bool,   // Automatically clean after scan
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            selected_drives: HashSet::new(),
            max_scan_threads: 4,
            scan_depth: 15,
            auto_clean: false,
        }
    }
}

// Structure to represent drive information
#[derive(Clone)]
struct DriveInfo {
    path: PathBuf,
    name: String,
    is_selected: bool,
}

#[derive(Debug, Clone)]
struct BackupFile {
    path: PathBuf,
    project_name: String,
    timestamp: String,
    file_size: u64,
    parsed_time: Option<(u32, u32)>, // (hours, minutes)
}

#[derive(Debug)]
enum ScanMessage {
    Progress(String, usize, usize), // (message, files_scanned, total_estimated)
    FoundBackup(String, BackupFile),
    Complete(usize),
}

impl BackupFile {
    fn new(path: PathBuf) -> Option<Self> {
        let file_name = path.file_name()?.to_str()?;
        
        // Only process .flp files
        if !file_name.ends_with(".flp") {
            return None;
        }

        // Parse the backup file format: "ProjectName (overwritten at XXhYY).flp"
        let re = Regex::new(r"^(.+) \(overwritten at (\d{1,2})h(\d{2})\)\.flp$").ok()?;
        let captures = re.captures(file_name)?;
        
        let project_name = captures.get(1)?.as_str().to_string();
        let hours: u32 = captures.get(2)?.as_str().parse().ok()?;
        let minutes: u32 = captures.get(3)?.as_str().parse().ok()?;
        let timestamp = format!("{}h{:02}", hours, minutes);
        
        let file_size = fs::metadata(&path).ok()?.len();
        
        Some(BackupFile {
            path,
            project_name,
            timestamp,
            file_size,
            parsed_time: Some((hours, minutes)),
        })
    }
    
    fn get_time_value(&self) -> u32 {
        if let Some((hours, minutes)) = self.parsed_time {
            hours * 60 + minutes // Convert to total minutes for easy comparison
        } else {
            0
        }
    }
}

impl FlBackupCleaner {
    fn new() -> Self {
        // Try to load settings from file
        let mut settings = load_settings().unwrap_or_default();
        let theme = settings.theme.clone();
        
        // Get available drives
        let available_drives = get_all_drives();
        let saved_selected_drives = settings.selected_drives.clone();
        
        // Clear selected drives (we'll rebuild it based on current available drives)
        settings.selected_drives.clear();
        
        // Convert drives to DriveInfo and set selection state based on saved settings
        let available_drives = available_drives.into_iter()
            .map(|path| {
                let name = path.display().to_string();
                let is_selected = saved_selected_drives.contains(&name);
                
                // Add to selected drives if it was previously selected or if there were no saved selections
                if is_selected || saved_selected_drives.is_empty() {
                    settings.selected_drives.insert(name.clone());
                }
                
                DriveInfo {
                    path,
                    name,
                    is_selected: is_selected || saved_selected_drives.is_empty(),
                }
            })
            .collect();
        
        Self {
            theme,
            error_messages: Vec::new(),
            detailed_progress: Vec::new(),
            settings_open: false,
            available_drives,
            settings,
            ..Default::default()
        }
    }
    
    fn start_scan(&mut self) {
        if self.is_scanning {
            return;
        }
        
        self.is_scanning = true;
        self.scan_complete = false;
        self.scan_status = "Starting scan...".to_string();
        self.scan_progress = String::new();
        self.found_backups.clear();
        self.total_files_found = 0;
        self.progress_percentage = 0.0;
        self.files_scanned = 0;
        self.total_files_estimated = 0;
        self.scan_start_time = Some(Instant::now());
        self.error_messages.clear();
        self.detailed_progress.clear();
        
        let (tx, rx) = mpsc::channel();
        self.scan_receiver = Some(rx);
        
        // Get selected drives
        let selected_drives: Vec<PathBuf> = self.available_drives.iter()
            .filter(|drive| self.settings.selected_drives.contains(&drive.name))
            .map(|drive| drive.path.clone())
            .collect();
            
        // Get scan settings
        let max_threads = self.settings.max_scan_threads;
        let max_depth = self.settings.scan_depth;
        let auto_clean = self.settings.auto_clean;
        
        // Spawn background thread for scanning
        thread::spawn(move || {
            let _ = tx.send(ScanMessage::Progress("Getting drive list...".to_string(), 0, 0));
            
            let drives = selected_drives;
            let _total_drives = drives.len();
            
            // We'll set a fixed estimate initially and adjust it as scanning proceeds
            // This avoids the progress bar showing more files than total
            let base_estimate_per_drive = 100000; // Lower initial estimate to avoid jumps
            let total_estimated_files = drives.len() * base_estimate_per_drive;
            
            let files_scanned = Arc::new(Mutex::new(0usize));
            let total_estimate = Arc::new(Mutex::new(total_estimated_files));
            let mut total_found = 0;
            let mut scan_threads = Vec::new();
            
            // Track which drives have been completed
            let completed_drives = Arc::new(Mutex::new(HashSet::new()));
            
            // Send initial estimate
            let _ = tx.send(ScanMessage::Progress(
                format!("Starting scan across {} drives", drives.len()),
                0,
                *total_estimate.lock().unwrap()
            ));
            
            // Create a shared set of already processed directories to avoid duplicates
            let processed_dirs = Arc::new(Mutex::new(HashSet::new()));
            
            for (_, drive) in drives.iter().enumerate() {
                let drive_name = drive.display().to_string();
                let tx_clone = tx.clone();
                let drive_clone = drive.clone();
                let files_scanned_clone = Arc::clone(&files_scanned);
                let total_estimate_clone = Arc::clone(&total_estimate);
                let processed_dirs_clone = Arc::clone(&processed_dirs);
                let completed_drives_clone = Arc::clone(&completed_drives);
                
                let _ = tx.send(ScanMessage::Progress(
                    format!("Scanning {}", drive_name),
                    *files_scanned.lock().unwrap(),
                    *total_estimate.lock().unwrap()
                ));
                
                // Create multiple scan threads per drive to improve performance
                // Use the configured number of threads per drive
                let handle = thread::spawn(move || {
                    let mut drive_found = 0;
                    
                    // Get top level directories to distribute work
                    let top_dirs = get_top_level_directories(&drive_clone);
                    
                    // Report if no top directories were found
                    if top_dirs.is_empty() {
                        let _ = tx_clone.send(ScanMessage::Progress(
                            format!("No scannable directories found on {}", drive_name),
                            *files_scanned_clone.lock().unwrap(),
                            *total_estimate_clone.lock().unwrap()
                        ));
                        
                        // Mark this drive as completed
                        completed_drives_clone.lock().unwrap().insert(drive_name.clone());
                        
                        return 0;
                    }
                    
                    let chunk_size = 1.max(top_dirs.len() / max_threads); // Use configured max threads
                    
                    let mut dir_threads = Vec::new();
                    let drive_files_scanned = Arc::new(Mutex::new(0usize));
                    
                    // Divide work among threads
                    for chunk in top_dirs.chunks(chunk_size) {
                        let tx_thread = tx_clone.clone();
                        let chunk_dirs = chunk.to_vec();
                        let files_scanned_thread = Arc::clone(&drive_files_scanned);
                        let files_scanned_global = Arc::clone(&files_scanned_clone);
                        let total_estimate_thread = Arc::clone(&total_estimate_clone);
                        let processed_dirs_thread = Arc::clone(&processed_dirs_clone);
                        
                        let thread_handle = thread::spawn(move || {
                            let mut dirs_found = 0;
                            
                            for dir in chunk_dirs {
                                // Update that we're scanning this directory
                                let _ = tx_thread.send(ScanMessage::Progress(
                                    format!("Scanning {}", dir.display()),
                                    *files_scanned_global.lock().unwrap(),
                                    *total_estimate_thread.lock().unwrap()
                                ));
                                
                                scan_directory(
                                    &dir, 
                                    &tx_thread, 
                                    &mut dirs_found, 
                                    files_scanned_thread.clone(),
                                    files_scanned_global.clone(),
                                    total_estimate_thread.clone(),
                                    processed_dirs_thread.clone(),
                                    max_depth  // Use configured max depth
                                ).unwrap_or_else(|e| {
                                    eprintln!("Error scanning {}: {}", dir.display(), e);
                                    // Send error to UI
                                    let _ = tx_thread.send(ScanMessage::Progress(
                                        format!("Error scanning {}: {}", dir.display(), e),
                                        *files_scanned_global.lock().unwrap(),
                                        *total_estimate_thread.lock().unwrap()
                                    ));
                                });
                            }
                            
                            dirs_found
                        });
                        
                        dir_threads.push(thread_handle);
                    }
                    
                    // Wait for all directory threads to complete
                    for thread_handle in dir_threads {
                        match thread_handle.join() {
                            Ok(found) => drive_found += found,
                            Err(e) => {
                                eprintln!("Thread join error: {:?}", e);
                                // Send error to UI
                                let _ = tx_clone.send(ScanMessage::Progress(
                                    format!("Thread error on {}", drive_name),
                                    *files_scanned_clone.lock().unwrap(),
                                    *total_estimate_clone.lock().unwrap()
                                ));
                            }
                        }
                    }
                    
                    // Report completion for this drive
                    let _ = tx_clone.send(ScanMessage::Progress(
                        format!("Completed scan of {}", drive_name),
                        *files_scanned_clone.lock().unwrap(),
                        *total_estimate_clone.lock().unwrap()
                    ));
                    
                    // Mark this drive as completed
                    completed_drives_clone.lock().unwrap().insert(drive_name);
                    
                    drive_found
                });
                
                scan_threads.push(handle);
            }
            
            // Wait for all scan threads to complete
            for handle in scan_threads {
                match handle.join() {
                    Ok(found) => total_found += found,
                    Err(e) => {
                        eprintln!("Drive thread join error: {:?}", e);
                    }
                }
            }
            
            // Verify all drives were completed
            let completed = completed_drives.lock().unwrap();
            if completed.len() < drives.len() {
                // Some drives didn't complete properly
                let mut missing_drives = Vec::new();
                for drive in drives.iter() {
                    let drive_name = drive.display().to_string();
                    if !completed.contains(&drive_name) {
                        missing_drives.push(drive_name.clone());
                    }
                }
                
                if !missing_drives.is_empty() {
                    let message = format!(
                        "Warning: Some drives were not fully scanned: {}",
                        missing_drives.join(", ")
                    );
                    let _ = tx.send(ScanMessage::Progress(
                        message,
                        *files_scanned.lock().unwrap(),
                        *total_estimate.lock().unwrap()
                    ));
                }
            }
            
            // Final update before completion
            let final_scanned = *files_scanned.lock().unwrap();
            let final_estimate = *total_estimate.lock().unwrap();
            let _ = tx.send(ScanMessage::Progress(
                format!("Scan completed. Processed {} files", final_scanned),
                final_scanned,
                final_estimate
            ));
            
            // Ensure we've scanned a reasonable number of files before completing
            // This helps prevent premature completion
            if final_scanned < 1000 {
                // Very few files scanned, might be an issue
                let _ = tx.send(ScanMessage::Progress(
                    format!("Warning: Only {} files were scanned. Some drives may have been skipped.", final_scanned),
                    final_scanned,
                    final_estimate
                ));
                
                // Add a small delay to ensure the message is seen
                thread::sleep(Duration::from_millis(500));
            }
            
            // Send completion message
            let _ = tx.send(ScanMessage::Complete(total_found));
            
            // Auto-clean if enabled
            if auto_clean && total_found > 0 {
                // Signal that auto-clean should happen
                // We'll handle this in the update_from_scan_messages method
                let _ = tx.send(ScanMessage::Progress(
                    "AUTO_CLEAN".to_string(),
                    final_scanned,
                    final_estimate
                ));
            }
        });
    }
    
    fn update_from_scan_messages(&mut self, ctx: &egui::Context) {
        let mut messages = Vec::new();
        let mut should_clear_receiver = false;
        let mut should_auto_clean = false;
        
        if let Some(receiver) = &self.scan_receiver {
            // Collect all available messages
            while let Ok(message) = receiver.try_recv() {
                if matches!(message, ScanMessage::Complete(_)) {
                    should_clear_receiver = true;
                }
                
                // Check for auto-clean signal
                if let ScanMessage::Progress(msg, _, _) = &message {
                    if msg == "AUTO_CLEAN" {
                        should_auto_clean = true;
                        continue; // Skip adding this message to the list
                    }
                }
                
                messages.push(message);
            }
        }
        
        // Process messages after borrowing is done
        for message in messages {
            match message {
                ScanMessage::Progress(msg, files_scanned, total_files) => {
                    self.scan_progress = msg.clone();
                    self.files_scanned = files_scanned;
                    
                    // Add to detailed progress log if it contains important info
                    if msg.contains("Error") || msg.contains("Found FL Studio") || 
                       msg.contains("Completed scan of") || msg.contains("Scan completed") {
                        // Limit the number of progress messages to avoid memory issues
                        if self.detailed_progress.len() > 100 {
                            self.detailed_progress.remove(0);
                        }
                        self.detailed_progress.push(msg.clone());
                    }
                    
                    // Record error messages separately
                    if msg.contains("Error") {
                        // Limit the number of error messages to avoid memory issues
                        if self.error_messages.len() > 50 {
                            self.error_messages.remove(0);
                        }
                        self.error_messages.push(msg);
                    }
                    
                    if total_files > 0 {
                        self.total_files_estimated = total_files;
                        self.progress_percentage = (files_scanned as f32 / total_files as f32) * 100.0;
                        
                        // Ensure progress doesn't exceed 100%
                        if self.progress_percentage > 100.0 {
                            self.progress_percentage = 100.0;
                        }
                    }
                    
                    ctx.request_repaint();
                }
                ScanMessage::FoundBackup(project_key, backup_file) => {
                    self.found_backups
                        .entry(project_key)
                        .or_insert_with(Vec::new)
                        .push(backup_file);
                    ctx.request_repaint();
                }
                ScanMessage::Complete(total_found) => {
                    self.total_files_found = total_found;
                    self.scan_status = format!(
                        "Scan complete! Found {} backup files in {} projects", 
                        self.total_files_found, 
                        self.found_backups.len()
                    );
                    self.scan_progress = String::new();
                    self.is_scanning = false;
                    self.scan_complete = true;
                    self.progress_percentage = 100.0;
                    self.files_scanned = self.total_files_estimated;
                    ctx.request_repaint();
                }
            }
        }
        
        // Clear receiver after processing all messages
        if should_clear_receiver {
            self.scan_receiver = None;
            
            // Auto-clean if needed
            if should_auto_clean {
                self.clean_backups();
            }
        }
    }
    
    fn clean_backups(&mut self) {
        self.deletion_status = "Cleaning backup files...".to_string();
        self.total_size_saved = 0;
        let mut deleted_count = 0;
        
        for (_project_key, backups) in &mut self.found_backups {
            if backups.len() <= 1 {
                continue; // Keep single backup files
            }
            
            // Sort by time (latest first)
            backups.sort_by(|a, b| b.get_time_value().cmp(&a.get_time_value()));
            
            // Keep the first (latest) backup, delete the rest
            for backup in backups.iter().skip(1) {
                match fs::remove_file(&backup.path) {
                    Ok(_) => {
                        self.total_size_saved += backup.file_size;
                        deleted_count += 1;
                    }
                    Err(e) => {
                        eprintln!("Failed to delete {}: {}", backup.path.display(), e);
                    }
                }
            }
            
            // Keep only the latest backup in our records
            backups.truncate(1);
        }
        
        self.deletion_status = format!(
            "Cleanup complete! Deleted {} files, saved {:.2} MB", 
            deleted_count, 
            self.total_size_saved as f64 / (1024.0 * 1024.0)
        );
    }
    
    fn refresh_drives(&mut self) {
        // Get current drives
        let current_drives = get_all_drives();
        let saved_selected_drives = self.settings.selected_drives.clone();
        
        // Clear selected drives (we'll rebuild it)
        self.settings.selected_drives.clear();
        
        // Update available drives list
        self.available_drives = current_drives.into_iter()
            .map(|path| {
                let name = path.display().to_string();
                
                // Check if this drive was previously selected
                let is_selected = saved_selected_drives.contains(&name);
                
                // Add to selected drives if it was previously selected
                if is_selected {
                    self.settings.selected_drives.insert(name.clone());
                }
                
                DriveInfo {
                    path,
                    name,
                    is_selected,
                }
            })
            .collect();
            
        // Save settings after refreshing drives
        if let Err(e) = save_settings(&self.settings) {
            eprintln!("Failed to save settings after drive refresh: {}", e);
        }
    }
    
    fn show_settings_modal(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        
        // Create a clone of settings to avoid borrow issues
        let mut settings = self.settings.clone();
        let mut drives = self.available_drives.clone();
        let mut should_close = false;
        let mut should_apply = false;
        let mut should_refresh = false;
        
        // Create a modal dialog for settings
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(false)
            .min_width(450.0)
            .min_height(400.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut self.settings_open)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add(egui::Label::new(
                        egui::RichText::new("FL Studio Backup Cleaner Settings")
                            .size(20.0)
                            .strong()
                            .color(egui::Color32::from_rgb(87, 190, 255))
                    ));
                });
                
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
                
                // Theme section
                ui.heading(egui::RichText::new("🎨 Appearance").size(16.0).strong());
                ui.add_space(5.0);
                
                // Theme selection with better styling
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().widgets.noninteractive.bg_fill)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Theme:");
                            if ui.radio_value(&mut settings.theme, Theme::Light, "☀️ Light").clicked() {
                                should_apply = true;
                            }
                            if ui.radio_value(&mut settings.theme, Theme::Dark, "🌙 Dark").clicked() {
                                should_apply = true;
                            }
                        });
                    });
                
                ui.add_space(15.0);
                
                // Drive selection section
                ui.heading(egui::RichText::new("💽 Drive Selection").size(16.0).strong());
                ui.add_space(5.0);
                
                // Description
                ui.label("Select which drives to scan for FL Studio backups:");
                
                // Refresh drives button with icon
                if ui.button(egui::RichText::new("🔄 Refresh Drive List").size(14.0)).clicked() {
                    should_refresh = true;
                }
                
                ui.add_space(5.0);
                
                // Drive selection in a scrollable area with better styling
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().widgets.noninteractive.bg_fill)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                            // Select/Deselect All buttons
                            ui.horizontal(|ui| {
                                if ui.button("Select All").clicked() {
                                    for drive in &mut drives {
                                        drive.is_selected = true;
                                        settings.selected_drives.insert(drive.name.clone());
                                    }
                                    should_apply = true;
                                }
                                if ui.button("Deselect All").clicked() {
                                    for drive in &mut drives {
                                        drive.is_selected = false;
                                        settings.selected_drives.remove(&drive.name);
                                    }
                                    should_apply = true;
                                }
                            });
                            
                            ui.separator();
                            
                            // Drive checkboxes
                            for drive in &mut drives {
                                let mut is_selected = settings.selected_drives.contains(&drive.name);
                                if ui.checkbox(&mut is_selected, &drive.name).changed() {
                                    if is_selected {
                                        settings.selected_drives.insert(drive.name.clone());
                                    } else {
                                        settings.selected_drives.remove(&drive.name);
                                    }
                                    drive.is_selected = is_selected;
                                    should_apply = true;
                                }
                            }
                        });
                    });
                
                ui.add_space(15.0);
                
                // Performance settings section
                ui.heading(egui::RichText::new("⚡ Performance").size(16.0).strong());
                ui.add_space(5.0);
                
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().widgets.noninteractive.bg_fill)
                    .show(ui, |ui| {
                        // Thread count slider
                        ui.horizontal(|ui| {
                            ui.label("Max threads per drive:");
                            if ui.add(egui::Slider::new(&mut settings.max_scan_threads, 1..=8).text("threads")).changed() {
                                should_apply = true;
                            }
                            
                            // Add tooltip
                            ui.label("ℹ️").on_hover_text(
                                "Higher values may increase scan speed but use more system resources.\n\
                                 Recommended: 4 threads for most systems."
                            );
                        });
                        
                        // Scan depth slider
                        ui.horizontal(|ui| {
                            ui.label("Max scan depth:");
                            if ui.add(egui::Slider::new(&mut settings.scan_depth, 5..=20).text("levels")).changed() {
                                should_apply = true;
                            }
                            
                            // Add tooltip
                            ui.label("ℹ️").on_hover_text(
                                "Maximum directory depth to scan.\n\
                                 Higher values ensure all files are found but may slow down scanning.\n\
                                 Recommended: 12-15 levels for most systems."
                            );
                        });
                    });
                
                ui.add_space(15.0);
                
                // Other settings section
                ui.heading(egui::RichText::new("🔧 Additional Options").size(16.0).strong());
                ui.add_space(5.0);
                
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().widgets.noninteractive.bg_fill)
                    .show(ui, |ui| {
                        // Auto-clean option
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut settings.auto_clean, "Automatically clean after scan").changed() {
                                should_apply = true;
                            }
                            
                            // Add tooltip
                            ui.label("ℹ️").on_hover_text(
                                "When enabled, the application will automatically clean up old backups\n\
                                 after the scan completes, keeping only the latest backup for each project."
                            );
                        });
                    });
                
                ui.add_space(20.0);
                
                // Bottom buttons
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        // Apply button
                        if ui.button(
                            egui::RichText::new("Apply")
                                .size(16.0)
                                .strong()
                        ).clicked() {
                            should_apply = true;
                            should_close = true;
                        }
                        
                        // Cancel button
                        if ui.button("Cancel").clicked() {
                            // Don't save settings
                            should_close = true;
                        }
                    });
                });
            });
            
        // Apply settings immediately when they change
        if should_apply {
            self.settings = settings.clone();
            self.available_drives = drives.clone();
            self.theme = self.settings.theme.clone();
            
            // Apply theme immediately
            match self.theme {
                Theme::Light => {
                    ctx.set_visuals(egui::Visuals::light());
                },
                Theme::Dark => {
                    ctx.set_visuals(egui::Visuals::dark());
                },
            }
            
            // Save settings to file
            if let Err(e) = save_settings(&self.settings) {
                eprintln!("Failed to save settings: {}", e);
            }
            
            // Request a repaint to show changes immediately
            ctx.request_repaint();
        }
        
        // Refresh drives if requested
        if should_refresh {
            self.refresh_drives();
            
            // Update the local copies for the UI
            drives = self.available_drives.clone();
            
            // Update selected drives in settings
            for drive in &drives {
                if drive.is_selected {
                    settings.selected_drives.insert(drive.name.clone());
                } else {
                    settings.selected_drives.remove(&drive.name);
                }
            }
        }
        
        // Check if window was closed or close button was clicked
        if !self.settings_open || should_close {
            // Final apply of settings if needed
            if should_apply || should_refresh {
                self.settings = settings;
                self.available_drives = drives;
                self.theme = self.settings.theme.clone();
                
                // Save settings to file
                if let Err(e) = save_settings(&self.settings) {
                    eprintln!("Failed to save settings: {}", e);
                }
            }
            
            // Ensure window is closed
            self.settings_open = false;
        }
    }
}

fn get_all_drives() -> Vec<PathBuf> {
    let mut drives = Vec::new();
    
    #[cfg(target_os = "windows")]
    {
        // On Windows, check drives A: through Z:
        for letter in b'A'..=b'Z' {
            let drive = format!("{}:\\", letter as char);
            let path = PathBuf::from(&drive);
            if path.exists() {
                drives.push(path);
            }
        }
    }
    
    #[cfg(not(target_os = "windows"))]
    {
        // On Unix-like systems, start from root and common mount points
        drives.push(PathBuf::from("/"));
        drives.push(PathBuf::from("/home"));
        drives.push(PathBuf::from("/Users")); // macOS
        drives.push(PathBuf::from("/mnt"));   // Linux mount points
        drives.push(PathBuf::from("/media")); // Linux removable media
    }
    
    drives
}

// Get the top-level directories in a drive to distribute work among threads
fn get_top_level_directories(drive: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    
    // Skip certain directories that are unlikely to contain FL Studio projects
    let skip_dirs = [
        "Windows", "Program Files", "Program Files (x86)", 
        "$Recycle.Bin", "System Volume Information", "ProgramData",
        "AppData", "PerfLogs", "Recovery", "$WINDOWS.~BT", "$WinREAgent",
        "Config.Msi", "Documents and Settings", "Intel", "$SysReset"
    ];
    
    if let Ok(entries) = fs::read_dir(drive) {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    let path = entry.path();
                    
                    if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                        // Skip system directories
                        if skip_dirs.iter().any(|&skip| dir_name.eq_ignore_ascii_case(skip)) 
                            || dir_name.starts_with(".") 
                            || dir_name.starts_with("$") {
                            continue;
                        }
                        
                        dirs.push(path);
                    }
                }
            }
        }
    }
    
    dirs
}

// Scan a directory for FL Studio backups
fn scan_directory(
    dir: &Path,
    tx: &mpsc::Sender<ScanMessage>,
    total_found: &mut usize,
    drive_scanned: Arc<Mutex<usize>>,
    global_scanned: Arc<Mutex<usize>>,
    total_estimate: Arc<Mutex<usize>>,
    processed_dirs: Arc<Mutex<HashSet<PathBuf>>>,
    max_depth: usize,  // Added max depth parameter
) -> Result<(), Box<dyn std::error::Error>> {
    // Skip certain directories that are unlikely to contain FL Studio projects
    let skip_dirs = [
        "Windows", "Program Files", "Program Files (x86)", 
        "$Recycle.Bin", "System Volume Information", "ProgramData",
        "AppData", "PerfLogs", "Recovery", "$WINDOWS.~BT", "$WinREAgent",
        "node_modules", "Config.Msi", "Documents and Settings", ".git", "Intel",
        "cache", "logs", "temp", "tmp", "obj", "bin", "debug", "release",
        "build", "dist", "target", "packages"
    ];
    
    // Check if directory has already been processed (for network paths that might appear multiple ways)
    {
        let mut processed = processed_dirs.lock().unwrap();
        if let Ok(canonical_path) = fs::canonicalize(dir) {
            if !processed.insert(canonical_path) {
                return Ok(());  // Already processed this directory
            }
        }
    }
    
    // Check if directory exists and is readable
    if !dir.exists() {
        return Ok(());  // Skip non-existent directories
    }
    
    // Use a custom error handler for walkdir to prevent it from failing on permission errors
    let walker = WalkDir::new(dir)
        .follow_links(false)
        .max_depth(max_depth) // Use configured max depth
        .into_iter()
        .filter_entry(|e| {
            // Skip system and hidden directories to speed up scanning
            if let Some(file_name) = e.file_name().to_str() {
                if skip_dirs.iter().any(|&dir| file_name.eq_ignore_ascii_case(dir)) {
                    return false;
                }
                
                // Skip hidden directories and typical development directories
                if file_name.starts_with(".") || file_name.starts_with("~") {
                    return false;
                }
            }
            true
        });
    
    // Use a time-based update system
    let mut most_recent_update = Instant::now();
    
    // Process entries with better error handling
    for entry_result in walker {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(err) => {
                // Just skip any errors (permissions, etc) and continue
                if most_recent_update.elapsed() > Duration::from_secs(5) {
                    most_recent_update = Instant::now();
                    let _ = tx.send(ScanMessage::Progress(
                        format!("Skipping inaccessible path in {}: {}", dir.display(), err),
                        *global_scanned.lock().unwrap(),
                        *total_estimate.lock().unwrap()
                    ));
                }
                continue;
            }
        };
        
        let path = entry.path();
        
        // Update files scanned counter
        if entry.file_type().is_file() {
            {
                let mut drive_count = drive_scanned.lock().unwrap();
                *drive_count += 1;
                
                let mut global_count = global_scanned.lock().unwrap();
                *global_count += 1;
                
                // Adjust the total estimate more gradually to prevent jumps
                let total = {
                    let mut total = total_estimate.lock().unwrap();
                    
                    // If we're approaching 50% of the current estimate, increase it
                    if *global_count > (*total * 50) / 100 {
                        // Increase by 50% of current estimate
                        *total = (*total * 150) / 100;
                    }
                    
                    // Ensure we never show more than 95% until complete
                    // This prevents the jump from ~40% to 100%
                    if (*global_count * 100) / *total > 95 && *global_count < *total {
                        *total = (*global_count * 105) / 100; // Keep at ~95% max
                    }
                    
                    *total
                };
                
                // Send progress update (but not too frequently)
                let now = Instant::now();
                if now.duration_since(most_recent_update) > Duration::from_millis(500) {
                    most_recent_update = now;
                    let _ = tx.send(ScanMessage::Progress(
                        format!("Scanning {}", dir.display()),
                        *global_count,
                        total
                    ));
                }
            }
        }
        
        // Fast path: specifically look for "Backup" folders
        if path.is_dir() && path.file_name().map_or(false, |name| name == "Backup") {
            // Check if parent directory looks like an FL Studio project folder
            if let Some(parent) = path.parent() {
                // Send update that we found a backup folder
                let _ = tx.send(ScanMessage::Progress(
                    format!("Found FL Studio backup folder: {}", path.display()),
                    *global_scanned.lock().unwrap(),
                    *total_estimate.lock().unwrap()
                ));
                
                scan_backup_folder(path, parent, tx, total_found)?;
            }
        }
    }
    
    Ok(())
}

// Speed up backup folder scanning
fn scan_backup_folder(
    backup_folder: &Path, 
    project_folder: &Path, 
    tx: &mpsc::Sender<ScanMessage>,
    total_found: &mut usize
) -> Result<(), Box<dyn std::error::Error>> {
    // Read all .flp files in the backup folder directly - no need for filtering
    if let Ok(entries) = fs::read_dir(backup_folder) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            
            // Only process .flp files
            if let Some(ext) = path.extension() {
                if ext != "flp" {
                    continue;
                }
                
                if let Some(backup_file) = BackupFile::new(path) {
                    // Use the full project folder path as the key to avoid conflicts
                    let project_key = format!("{}#{}", 
                                            project_folder.display(), 
                                            backup_file.project_name);
                    
                    let _ = tx.send(ScanMessage::FoundBackup(project_key, backup_file));
                    *total_found += 1;
                }
            }
        }
    }
    Ok(())
}

// Get settings file path
fn get_settings_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("flcleaner");
    fs::create_dir_all(&path).ok();
    path.push("settings.json");
    path
}

// Load settings from file
fn load_settings() -> Result<Settings, Box<dyn std::error::Error>> {
    let path = get_settings_path();
    
    // Check if file exists
    if !path.exists() {
        return Ok(Settings::default());
    }
    
    // Read file
    let mut file = fs::File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    
    // Deserialize JSON
    let settings: Settings = serde_json::from_str(&contents)?;
    
    Ok(settings)
}

// Save settings to file
fn save_settings(settings: &Settings) -> Result<(), Box<dyn std::error::Error>> {
    let path = get_settings_path();
    
    // Serialize settings to JSON
    let json = serde_json::to_string_pretty(settings)?;
    
    // Write to file
    let mut file = fs::File::create(path)?;
    file.write_all(json.as_bytes())?;
    
    Ok(())
}

impl eframe::App for FlBackupCleaner {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Set the theme
        match self.theme {
            Theme::Light => {
                ctx.set_visuals(egui::Visuals::light());
            },
            Theme::Dark => {
                ctx.set_visuals(egui::Visuals::dark());
            },
        }
        
        // Process scan messages first
        self.update_from_scan_messages(ctx);
        
        // Show settings modal if open
        self.show_settings_modal(ctx);
        
        egui::CentralPanel::default().show(ctx, |ui| {
            // Header with settings button
            ui.horizontal(|ui| {
                // Add flexible space at the beginning for centering
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center).with_main_justify(true), |ui| {
                    ui.add(egui::Label::new(
                        egui::RichText::new("FL Studio Backup Cleaner")
                            .size(24.0)
                            .strong()
                            .color(egui::Color32::from_rgb(87, 190, 255))
                    ));
                });
                
                // Replace theme toggle with settings button
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⚙️ Settings").clicked() {
                        self.settings_open = true;
                    }
                });
            });
            ui.separator();
            
            // Center the rest of the content
            ui.vertical_centered(|ui| {
                // App description with better styling
                ui.add(egui::Label::new(
                    egui::RichText::new("This tool will scan all your drives for FL Studio backup folders and clean up old backup files, keeping only the latest backup for each project.")
                        .size(14.0)
                ));
                ui.add_space(15.0);
                
                // Styled scan button with icon
                let scan_btn = egui::Button::new(
                    egui::RichText::new("🔍 Scan for FL Studio Backups")
                        .size(18.0)
                        .strong()
                )
                .min_size(egui::vec2(280.0, 40.0))
                .fill(egui::Color32::from_rgb(60, 130, 200));
                
                ui.add_enabled_ui(!self.is_scanning, |ui| {
                    if ui.add(scan_btn).clicked() {
                        self.start_scan();
                    }
                });
                
                // Show scan status and progress with better styling
                if !self.scan_status.is_empty() {
                    ui.add_space(5.0);
                    ui.label(egui::RichText::new(&self.scan_status).size(16.0));
                }
                
                if self.is_scanning {
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        if !self.scan_progress.is_empty() {
                            ui.label(egui::RichText::new(&self.scan_progress).size(14.0));
                        } else {
                            ui.label("Scanning... This may take a while depending on your drive size.");
                        }
                    });
                    
                    // Enhanced progress bar with color gradient
                    if self.total_files_estimated > 0 {
                        ui.add_space(5.0);
                        
                        // Calculate progress with a cap to prevent jumps
                        let mut progress = (self.files_scanned as f32 / self.total_files_estimated as f32) * 100.0;
                        
                        // Cap progress at 95% until scan is complete
                        if progress > 95.0 && !self.scan_complete {
                            progress = 95.0;
                        }
                        
                        self.progress_percentage = progress;
                        let progress_fraction = progress / 100.0;
                        
                        // Choose color based on progress
                        let progress_color = if progress_fraction < 0.3 {
                            egui::Color32::from_rgb(255, 100, 100) // Red-ish for early progress
                        } else if progress_fraction < 0.7 {
                            egui::Color32::from_rgb(255, 180, 60) // Orange-ish for mid progress
                        } else {
                            egui::Color32::from_rgb(100, 200, 100) // Green-ish for near completion
                        };
                        
                        ui.add(
                            egui::ProgressBar::new(progress_fraction)
                                .show_percentage()
                                .animate(true)
                                .fill(progress_color)
                        );
                        
                        // Show time elapsed with better formatting
                        if let Some(start_time) = self.scan_start_time {
                            ui.add_space(2.0);
                            let elapsed = start_time.elapsed().as_secs();
                            ui.label(
                                egui::RichText::new(format!(
                                    "Time elapsed: {}m {}s",
                                    elapsed / 60,
                                    elapsed % 60
                                ))
                                .size(14.0)
                            );
                        }
                        
                        // Show detailed progress information in a collapsible section
                        if !self.detailed_progress.is_empty() {
                            ui.add_space(5.0);
                            egui::CollapsingHeader::new("Scan Progress Details")
                                .id_source("progress_details")
                                .show(ui, |ui| {
                                    egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                                        for message in self.detailed_progress.iter().rev() {
                                            // Color code by message type
                                            let text = if message.contains("Error") {
                                                egui::RichText::new(message).color(egui::Color32::from_rgb(255, 100, 100))
                                            } else if message.contains("Found FL Studio") {
                                                egui::RichText::new(message).color(egui::Color32::from_rgb(100, 255, 100))
                                            } else if message.contains("Completed scan") {
                                                egui::RichText::new(message).color(egui::Color32::from_rgb(100, 200, 255))
                                            } else {
                                                egui::RichText::new(message)
                                            };
                                            ui.label(text);
                                        }
                                    });
                                });
                        }
                        
                        // Show error messages if any
                        if !self.error_messages.is_empty() {
                            ui.add_space(5.0);
                            egui::CollapsingHeader::new(
                                egui::RichText::new(format!("Errors ({})", self.error_messages.len()))
                                    .color(egui::Color32::from_rgb(255, 100, 100))
                            )
                            .id_source("error_messages")
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                                    for message in self.error_messages.iter().rev() {
                                        ui.label(egui::RichText::new(message).color(egui::Color32::from_rgb(255, 100, 100)));
                                    }
                                });
                            });
                        }
                    }
                } else if self.scan_complete {
                    // Show completion status when scan is complete
                    ui.add_space(5.0);
                    let progress_color = egui::Color32::from_rgb(100, 200, 100); // Green for completion
                    ui.add(
                        egui::ProgressBar::new(1.0)
                            .text("Complete")
                            .fill(progress_color)
                    );
                    
                    // Show detailed progress information if available
                    if !self.detailed_progress.is_empty() {
                        ui.add_space(5.0);
                        egui::CollapsingHeader::new("Scan Details")
                            .id_source("complete_details")
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                                    for message in self.detailed_progress.iter().rev() {
                                        // Color code by message type
                                        let text = if message.contains("Error") {
                                            egui::RichText::new(message).color(egui::Color32::from_rgb(255, 100, 100))
                                        } else if message.contains("Found FL Studio") {
                                            egui::RichText::new(message).color(egui::Color32::from_rgb(100, 255, 100))
                                        } else if message.contains("Completed scan") {
                                            egui::RichText::new(message).color(egui::Color32::from_rgb(100, 200, 255))
                                        } else {
                                            egui::RichText::new(message)
                                        };
                                        ui.label(text);
                                    }
                                });
                            });
                    }
                }
                
                // Show current findings with improved styling
                let current_projects = self.found_backups.len();
                let current_files = self.found_backups.values().map(|v| v.len()).sum::<usize>();
                if current_files > 0 {
                    ui.add_space(5.0);
                    ui.label(
                        egui::RichText::new(format!("Found: {} files in {} projects", 
                            current_files, current_projects))
                        .size(14.0)
                        .color(egui::Color32::from_rgb(180, 230, 180))
                    );
                }
                
                ui.add_space(10.0);
                
                // Show found backups with enhanced UI
                if self.scan_complete && !self.found_backups.is_empty() {
                    ui.separator();
                    ui.add_space(5.0);
                    ui.heading(egui::RichText::new("Found Backup Files:").size(20.0).strong());
                    
                    // Show summary
                    let total_backups: usize = self.found_backups.values().map(|v| v.len()).sum();
                    let projects_with_multiple_backups = self.found_backups.values()
                        .filter(|backups| backups.len() > 1)
                        .count();
                    
                    // Enhanced summary card
                    ui.add_space(10.0);
                    egui::Frame::group(ui.style())
                        .fill(egui::Color32::from_rgb(40, 45, 55))
                        .show(ui, |ui| {
                            // Use a simpler layout with three separate rows
                            ui.vertical_centered(|ui| {
                                // First row: Projects found
                                ui.add_space(5.0);
                                ui.label(egui::RichText::new("Projects found:").strong().color(egui::Color32::from_rgb(200, 200, 200)));
                                ui.label(egui::RichText::new(format!("{}", self.found_backups.len())).size(28.0).color(egui::Color32::from_rgb(120, 210, 255)));
                                ui.add_space(5.0);
                                
                                // Second row: Total backup files
                                ui.label(egui::RichText::new("Total backup files:").strong().color(egui::Color32::from_rgb(200, 200, 200)));
                                ui.label(egui::RichText::new(format!("{}", total_backups)).size(28.0).color(egui::Color32::from_rgb(120, 210, 255)));
                                ui.add_space(5.0);
                                
                                // Third row: Projects with multiple backups
                                ui.label(egui::RichText::new("Projects with multiple backups:").strong().color(egui::Color32::from_rgb(200, 200, 200)));
                                ui.label(egui::RichText::new(format!("{}", projects_with_multiple_backups)).size(28.0).color(egui::Color32::from_rgb(120, 210, 255)));
                                ui.add_space(5.0);
                            });
                        });
                    
                    ui.add_space(15.0);
                    
                    // Enhanced clean button
                    if projects_with_multiple_backups > 0 {
                        let clean_btn = egui::Button::new(
                            egui::RichText::new("🧹 Clean Old Backups (Keep Latest Only)")
                                .size(18.0)
                                .strong()
                        )
                        .min_size(egui::vec2(320.0, 40.0))
                        .fill(egui::Color32::from_rgb(80, 170, 80));
                        
                        if ui.add(clean_btn).clicked() {
                            self.clean_backups();
                        }
                    }
                    
                    // Show deletion status with enhanced styling
                    if !self.deletion_status.is_empty() {
                        ui.add_space(10.0);
                        ui.label(egui::RichText::new(&self.deletion_status).size(16.0).color(egui::Color32::from_rgb(150, 220, 150)));
                        
                        // Enhanced space saved display
                        if self.total_size_saved > 0 {
                            ui.add_space(10.0);
                            let size_mb = self.total_size_saved as f64 / (1024.0 * 1024.0);
                            
                            egui::Frame::group(ui.style())
                                .fill(egui::Color32::from_rgb(40, 60, 40))
                                .show(ui, |ui| {
                                    ui.vertical_centered(|ui| {
                                        ui.label(egui::RichText::new("Space Saved").strong().color(egui::Color32::from_rgb(200, 255, 200)));
                                        ui.label(egui::RichText::new(format!("{:.2} MB", size_mb)).size(32.0).color(egui::Color32::from_rgb(120, 255, 120)));
                                    });
                                });
                        }
                    }
                    
                    ui.add_space(15.0);
                    
                    // Enhanced project details display
                    ui.heading(egui::RichText::new("Project Details:").size(18.0).strong());
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for (project_key, backups) in &self.found_backups {
                            let project_name = project_key.split('#').last().unwrap_or("Unknown");
                            let project_path = project_key.split('#').next().unwrap_or("Unknown path");
                            
                            egui::Frame::group(ui.style())
                                .fill(egui::Color32::from_rgb(35, 40, 50))
                                .show(ui, |ui| {
                                    // Project header
                                    ui.label(egui::RichText::new(format!("Project: {}", project_name)).strong().size(16.0).color(egui::Color32::from_rgb(150, 200, 255)));
                                    ui.label(egui::RichText::new(format!("Path: {}", project_path)).size(13.0).color(egui::Color32::from_rgb(180, 180, 180)));
                                    ui.label(egui::RichText::new(format!("Backups: {}", backups.len())).color(egui::Color32::from_rgb(200, 200, 200)));
                                    
                                    // Add some space before backups
                                    ui.add_space(5.0);
                                    
                                    // Display backups with indentation
                                    for backup in backups {
                                        ui.horizontal(|ui| {
                                            ui.add_space(20.0); // Indentation
                                            let size_kb = backup.file_size as f64 / 1024.0;
                                            ui.label(
                                                egui::RichText::new(format!("└─ {} ({:.1} KB)", backup.timestamp, size_kb))
                                                    .color(egui::Color32::from_rgb(220, 220, 220))
                                            );
                                        });
                                    }
                                });
                            ui.add_space(5.0);
                        }
                    });
                } else if self.scan_complete {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("No FL Studio backup files found on your system.").size(16.0));
                }
            }); // End of vertical_centered
        });
    }
    
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        // Save settings when app closes
        if let Err(e) = save_settings(&self.settings) {
            eprintln!("Failed to save settings on exit: {}", e);
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    let app = FlBackupCleaner::new();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([600.0, 400.0]),
        ..Default::default()
    };
    
    eframe::run_native(
        "FL Studio Backup Cleaner",
        options,
        Box::new(|_cc| Box::new(app)),
    )
}
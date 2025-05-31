// Cargo.toml dependencies needed:
// [dependencies]
// eframe = { version = "0.24", features = ["default"] }
// walkdir = "2.3"
// regex = "1.7"
// chrono = { version = "0.4", features = ["serde"] }
// tokio = { version = "1.0", features = ["full"] }
// 
// [target.'cfg(windows)'.dependencies]
// winapi = { version = "0.3", features = ["winuser", "windef", "shellapi", "objbase", "combaseapi"] }

use eframe::egui;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use walkdir::WalkDir;
use regex::Regex;

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
    Progress(String),
    FoundBackup(String, BackupFile),
    Complete(usize),
    Error(String),
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
        Self::default()
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
        
        let (tx, rx) = mpsc::channel();
        self.scan_receiver = Some(rx);
        
        // Spawn background thread for scanning
        thread::spawn(move || {
            let _ = tx.send(ScanMessage::Progress("Getting drive list...".to_string()));
            
            let drives = get_all_drives();
            let total_drives = drives.len();
            
            let mut total_found = 0;
            
            for (i, drive) in drives.iter().enumerate() {
                let drive_name = drive.display().to_string();
                let _ = tx.send(ScanMessage::Progress(
                    format!("Scanning drive {} ({}/{})...", drive_name, i + 1, total_drives)
                ));
                
                if let Err(e) = scan_drive(&drive, &tx, &mut total_found) {
                    let _ = tx.send(ScanMessage::Error(format!("Error scanning {}: {}", drive_name, e)));
                }
            }
            
            let _ = tx.send(ScanMessage::Complete(total_found));
        });
    }
    
    fn update_from_scan_messages(&mut self, ctx: &egui::Context) {
        let mut messages = Vec::new();
        let mut should_clear_receiver = false;
        
        if let Some(receiver) = &self.scan_receiver {
            // Collect all available messages
            while let Ok(message) = receiver.try_recv() {
                if matches!(message, ScanMessage::Complete(_)) {
                    should_clear_receiver = true;
                }
                messages.push(message);
            }
        }
        
        // Process messages after borrowing is done
        for message in messages {
            match message {
                ScanMessage::Progress(msg) => {
                    self.scan_progress = msg;
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
                    ctx.request_repaint();
                }
                ScanMessage::Error(msg) => {
                    eprintln!("Scan error: {}", msg);
                    ctx.request_repaint();
                }
            }
        }
        
        // Clear receiver after processing all messages
        if should_clear_receiver {
            self.scan_receiver = None;
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

fn scan_drive(drive: &Path, tx: &mpsc::Sender<ScanMessage>, total_found: &mut usize) -> Result<(), Box<dyn std::error::Error>> {
    // Walk through all directories looking for "Backup" folders
    for entry in WalkDir::new(drive)
        .follow_links(false)
        .max_depth(10) // Limit depth to avoid infinite recursion
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        
        // Check if this is a "Backup" folder
        if path.is_dir() && path.file_name().map_or(false, |name| name == "Backup") {
            // Check if parent directory looks like an FL Studio project folder
            if let Some(parent) = path.parent() {
                scan_backup_folder(path, parent, tx, total_found)?;
            }
        }
    }
    Ok(())
}

fn scan_backup_folder(
    backup_folder: &Path, 
    project_folder: &Path, 
    tx: &mpsc::Sender<ScanMessage>,
    total_found: &mut usize
) -> Result<(), Box<dyn std::error::Error>> {
    // Read all .flp files in the backup folder
    if let Ok(entries) = fs::read_dir(backup_folder) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
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
    Ok(())
}

impl eframe::App for FlBackupCleaner {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process scan messages first
        self.update_from_scan_messages(ctx);
        
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("FL Studio Backup Cleaner");
            ui.separator();
            
            ui.label("This tool will scan all your drives for FL Studio backup folders and clean up old backup files, keeping only the latest backup for each project.");
            ui.add_space(10.0);
            
            // Scan button
            if ui.button("🔍 Scan for FL Studio Backups").clicked() && !self.is_scanning {
                self.start_scan();
            }
            
            // Show scan status and progress
            if !self.scan_status.is_empty() {
                ui.label(&self.scan_status);
            }
            
            if self.is_scanning {
                ui.horizontal(|ui| {
                    ui.spinner();
                    if !self.scan_progress.is_empty() {
                        ui.label(&self.scan_progress);
                    } else {
                        ui.label("Scanning... This may take a while depending on your drive size.");
                    }
                });
                
                // Show current progress
                let current_projects = self.found_backups.len();
                let current_files = self.found_backups.values().map(|v| v.len()).sum::<usize>();
                if current_files > 0 {
                    ui.label(format!("Found so far: {} files in {} projects", current_files, current_projects));
                }
            }
            
            ui.add_space(10.0);
            
            // Show found backups
            if self.scan_complete && !self.found_backups.is_empty() {
                ui.separator();
                ui.heading("Found Backup Files:");
                
                // Show summary
                let total_backups: usize = self.found_backups.values().map(|v| v.len()).sum();
                let projects_with_multiple_backups = self.found_backups.values()
                    .filter(|backups| backups.len() > 1)
                    .count();
                
                ui.label(format!("Projects found: {}", self.found_backups.len()));
                ui.label(format!("Total backup files: {}", total_backups));
                ui.label(format!("Projects with multiple backups: {}", projects_with_multiple_backups));
                
                ui.add_space(10.0);
                
                // Clean button
                if projects_with_multiple_backups > 0 {
                    if ui.button("🧹 Clean Old Backups (Keep Latest Only)").clicked() {
                        self.clean_backups();
                    }
                }
                
                // Show deletion status
                if !self.deletion_status.is_empty() {
                    ui.label(&self.deletion_status);
                }
                
                ui.add_space(10.0);
                
                // Show detailed list in a scrollable area
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for (project_key, backups) in &self.found_backups {
                        let project_name = project_key.split('#').last().unwrap_or("Unknown");
                        let project_path = project_key.split('#').next().unwrap_or("Unknown path");
                        
                        ui.group(|ui| {
                            ui.strong(format!("Project: {}", project_name));
                            ui.label(format!("Path: {}", project_path));
                            ui.label(format!("Backups: {}", backups.len()));
                            
                            for backup in backups {
                                ui.label(format!("  └─ {} ({:.1} KB)", 
                                               backup.timestamp, 
                                               backup.file_size as f64 / 1024.0));
                            }
                        });
                        ui.add_space(5.0);
                    }
                });
            } else if self.scan_complete {
                ui.label("No FL Studio backup files found on your system.");
            }
        });
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
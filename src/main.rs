use chrono::{Local, LocalResult, TimeZone};
use eframe::egui::{self, Color32, ScrollArea};
extern crate ini;
use ini::Ini;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;
use directories_next::ProjectDirs;
use notify::{RecommendedWatcher, Watcher};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlayedGame {
    name: String,
    id: String,
    total_achievements: usize,
    unlocked_achievements: usize,
    active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserProfile {
    username: String,
    #[serde(default = "default_user_id")]
    user_id: String,
    played_games: Vec<PlayedGame>,
}

fn default_user_id() -> String {
    generate_random_id()
}

fn generate_random_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..17)
        .map(|_| (rng.gen_range(0..=9)).to_string())
        .collect()
}

fn is_valid_id(id: &str) -> bool {
    id.len() == 17 && id.chars().all(|c| c.is_ascii_digit())
}

#[derive(PartialEq)]
enum CurrentView {
    Home,
    Game,
}

fn find_save_directory(root: &PathBuf, game_id: &str) -> Option<PathBuf> {
    let candidates = vec![
        root.join(game_id),
        root.join("GSE Saves").join(game_id),
        root.join("gse saves").join(game_id),
        root.join("GSE_SAVES").join(game_id),
        root.join("..").join("GSE Saves").join(game_id),
        root.to_path_buf(),
    ];

    candidates.into_iter().find(|path| path.exists())
}

fn find_steam_settings_path(game_folder: &PathBuf) -> Option<PathBuf> {
    if game_folder.file_name().and_then(|n| n.to_str()) == Some("steam_settings") {
        return Some(game_folder.clone());
    }

    for entry in WalkDir::new(game_folder)
        .min_depth(1)
        .max_depth(10)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_name() == "steam_settings" && entry.file_type().is_dir() {
            return Some(entry.into_path());
        }
    }

    Some(game_folder.clone())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Achievement {
    name: String,
    #[serde(rename = "displayName")]
    display_name: String,
    description: String,
    hidden: u8,
    icon: String,
    #[serde(rename = "icon_gray")]
    icon_gray: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EarnedAchievement {
    earned: bool,
    earned_time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GameConfig {
    name: String,
    id: String,
    game_folder: PathBuf,
    save_folders: Vec<PathBuf>,
    #[serde(default)]
    total_achievements: usize,
    #[serde(default)]
    unlocked_achievements: usize,
}

struct AchievementApp {
    games: Vec<GameConfig>,
    current_game: Option<usize>,
    current_view: CurrentView,
    new_game: GameConfig,
    achievements: HashMap<String, Achievement>,
    earned: HashMap<String, EarnedAchievement>,
    images: HashMap<String, egui::TextureHandle>,
    watchers: Vec<RecommendedWatcher>,
    show_config_window: bool,
    user_profile: UserProfile,
    editing_username: bool,
    editing_id: bool,
    #[cfg(not(target_arch = "wasm32"))]
    receiver: Option<mpsc::Receiver<()>>,
    error_message: Option<String>,
    editing_game: Option<usize>,
    need_focus: bool,
}

impl AchievementApp {
    fn load_config() -> Vec<GameConfig> {
        ProjectDirs::from("com", "AchievementViewer", "Achievement Viewer")
            .map(|proj_dirs| {
                let config_dir = proj_dirs.config_dir();
                std::fs::create_dir_all(config_dir).ok();
                config_dir.join("config.json")
            })
            .and_then(|config_path| {
                std::fs::File::open(config_path)
                    .ok()
                    .and_then(|file| serde_json::from_reader(file).ok())
            })
            .unwrap_or_default()
    }

    fn load_profile() -> UserProfile {
        ProjectDirs::from("com", "AchievementViewer", "Achievement Viewer")
            .and_then(|proj_dirs| {
                let profile_path = proj_dirs.config_dir().join("profile.json");
                std::fs::read_to_string(profile_path)
                    .ok()
                    .and_then(|contents| serde_json::from_str(&contents).ok())
            })
            .unwrap_or_else(|| UserProfile {
                username: "Newbie".to_string(),
                user_id: generate_random_id(),
                played_games: Vec::new(),
            })
    }

    fn save_config(&self) {
        if let Some(proj_dirs) = ProjectDirs::from("com", "AchievementViewer", "Achievement Viewer") {
            let config_dir = proj_dirs.config_dir();
            let _ = std::fs::create_dir_all(config_dir);
            let config_path = config_dir.join("config.json");
            let _ = serde_json::to_writer_pretty(
                std::fs::File::create(config_path).unwrap(),
                &self.games
            );
        }
    }

    fn save_profile(&self) {
        if let Some(proj_dirs) = ProjectDirs::from("com", "AchievementViewer", "Achievement Viewer") {
            let config_dir = proj_dirs.config_dir();
            let _ = std::fs::create_dir_all(config_dir);
            let profile_path = config_dir.join("profile.json");
            let _ = serde_json::to_writer_pretty(
                std::fs::File::create(profile_path).unwrap(),
                &self.user_profile
            );
        }
    }

    fn update_username_configs(&self) {
        let username = &self.user_profile.username;
        let steam_id = &self.user_profile.user_id;

        for game in &self.games {
            if let Some(settings_path) = find_steam_settings_path(&game.game_folder) {
                let config_path = settings_path.join("configs.user.ini");

                let mut conf = match Ini::load_from_file(&config_path) {
                    Ok(existing) => existing,
                    Err(_) => Ini::new(),
                };

                conf.with_section(Some("user::general".to_string()))
                    .set("account_name", username)
                    .set("account_steamid", steam_id);

                if let Err(e) = conf.write_to_file(&config_path) {
                    eprintln!("Failed to write config for {}: {}", game.name, e);
                }
            }
        }
    }

    fn remove_game(&mut self, index: usize) {
        if index < self.games.len() {
            self.games.remove(index);
            self.save_config();
            if self.current_game == Some(index) {
                self.current_game = None;
                self.achievements.clear();
                self.earned.clear();
                self.images.clear();
            }
        }
    }

    fn validate_game_config(game: &GameConfig) -> Result<(), String> {
        if game.name.is_empty() {
            return Err("Game name cannot be empty".into());
        }
        if game.id.is_empty() {
            return Err("Game ID cannot be empty".into());
        }
        if !game.game_folder.exists() {
            return Err("Game folder does not exist".into());
        }

        let mut has_valid_save_path = false;
        for save_folder in &game.save_folders {
            if find_save_directory(save_folder, &game.id).is_some() {
                has_valid_save_path = true;
                break;
            }
        }

        if !has_valid_save_path {
            return Err("Could not find GSE Saves structure in any save folders".into());
        }

        let settings_path = find_steam_settings_path(&game.game_folder)
            .ok_or("Could not find steam_settings directory")?;
        
        if !settings_path.join("achievements.json").exists() {
            return Err("Game folder must contain achievements.json".into());
        }

        Ok(())
    }

    fn setup_watchers(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (sender, receiver) = mpsc::channel();
            self.receiver = Some(receiver);
            self.watchers.clear();

            for game in &self.games {
                let mut paths_to_watch = vec![game.game_folder.clone()];
                paths_to_watch.extend(game.save_folders.iter().cloned());

                for path in paths_to_watch {
                    let sender_clone = sender.clone();
                    let _game_id = game.id.clone();
                
                    let mut watcher = notify::recommended_watcher(
                        move |res: notify::Result<notify::Event>| {
                            if let Ok(event) = res {
                                if matches!(event.kind, notify::event::EventKind::Modify(_)) {
                                    std::thread::sleep(std::time::Duration::from_millis(500));
                                    let _ = sender_clone.send(());
                                }
                            }
                        }
                    ).unwrap();

                    if watcher.watch(&path, notify::RecursiveMode::Recursive).is_ok() {
                        self.watchers.push(watcher);
                    }
                }
            }
        }
    }

    fn load_earned_data(&mut self) {
        self.earned.clear();

        if let Some(game_index) = self.current_game {
            let game = &mut self.games[game_index];
            let mut check_paths = game.save_folders.clone();
            check_paths.push(game.game_folder.clone());

            for save_root in check_paths.iter() {
                if let Some(save_path) = find_save_directory(save_root, &game.id) {
                    let earned_path = save_path.join("achievements.json");
                    for _ in 0..5 {
                        match std::fs::read_to_string(&earned_path) {
                            Ok(contents) => {
                                if let Ok(earned) = serde_json::from_str(&contents) {
                                    self.earned = earned;
                                    game.unlocked_achievements = self.earned.values()
                                        .filter(|ea| ea.earned)
                                        .count();
                                    return;
                                }
                            },
                            Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
                        }
                    }
                }
            }
        }
        self.earned.clear();
        if let Some(game_index) = self.current_game {
            let game = &mut self.games[game_index];
            game.unlocked_achievements = 0;
        }
    }

    fn update_played_games(&mut self) {
        let _active_ids: HashSet<_> = self.games.iter().map(|g| &g.id).collect();
        
        for played in &mut self.user_profile.played_games {
            if let Some(game) = self.games.iter().find(|g| g.id == played.id) {
                played.unlocked_achievements = game.unlocked_achievements;
                played.total_achievements = game.total_achievements;
                played.active = true;
            } else {
                played.active = false;
            }
        }

        for game in &self.games {
            if !self.user_profile.played_games.iter().any(|p| p.id == game.id) {
                self.user_profile.played_games.push(PlayedGame {
                    name: game.name.clone(),
                    id: game.id.clone(),
                    total_achievements: game.total_achievements,
                    unlocked_achievements: game.unlocked_achievements,
                    active: true,
                });
            }
        }

        let mut seen = HashSet::new();
        self.user_profile.played_games.retain(|p| seen.insert(p.id.clone()));
    }

    fn load_game_data(&mut self, ctx: &egui::Context) {
        self.error_message = None;
        self.achievements.clear();
        self.images.clear();

        if let Some(game_index) = self.current_game {
            let game = &mut self.games[game_index];
            let base_path = find_steam_settings_path(&game.game_folder).unwrap_or_else(|| game.game_folder.clone());

            let config_path = base_path.join("configs.user.ini");
            if !config_path.exists() {
                let mut ini = Ini::new();
                ini.with_section(Some("user::general".to_string()))
                    .set("account_name", &self.user_profile.username)
                    .set("account_steamid", &self.user_profile.user_id);
            
                if let Err(e) = ini.write_to_file(&config_path) {
                    eprintln!("Failed to create config: {}", e);
                }
            }

            if let Ok(json_str) = std::fs::read_to_string(base_path.join("achievements.json")) {
                match serde_json::from_str::<HashMap<String, Achievement>>(&json_str)
                    .or_else(|_| serde_json::from_str::<Vec<Achievement>>(&json_str).map(|vec| vec.into_iter().map(|a| (a.name.clone(), a)).collect())) 
                {
                    Ok(achievements) => {
                        self.achievements = achievements;
                        game.total_achievements = self.achievements.len();
                    },
                    Err(e) => self.error_message = Some(format!("JSON Error: {}", e)),
                }
            }

            let image_folder = base_path.join("achievement_images");
            self.load_images(ctx, &image_folder);
            self.load_earned_data();
            ctx.request_repaint();
        }
    }

    fn display_achievement(&self, ui: &mut egui::Ui, ach: &Achievement) {
        let earned_entry = self.earned.get(&ach.name);
        let earned = earned_entry.map(|ea| ea.earned).unwrap_or(false);
        let image_name = if earned { ach.icon.to_lowercase() } else { ach.icon_gray.to_lowercase() };
        let texture = self.images.get(&image_name);

        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if let Some(texture) = texture {
                    ui.add(egui::Image::new(texture).max_size(egui::vec2(64.0, 64.0)));
                } else {
                    ui.label("?");
                }

                ui.vertical(|ui| {
                    ui.strong(&ach.display_name);
                    let (description, show_tooltip) = if !earned && ach.hidden == 1 {
                        ("••••••••••".to_owned(), true)
                    } else {
                        (ach.description.clone(), false)
                    };
            
                    let response = ui.label(description);
                    if show_tooltip {
                        response.on_hover_text(&ach.description);
                    }

                    if let Some(ea) = earned_entry {
                        if ea.earned {
                            if let LocalResult::Single(datetime) = Local.timestamp_opt(ea.earned_time as i64, 0) {
                                ui.label(format!("Unlocked on {}", datetime.format("%m/%d/%Y - %I:%M:%S %p")));
                            }
                        }
                    }
                });
            });
        });
    }

    fn load_images(&mut self, ctx: &egui::Context, image_folder: &PathBuf) {
        self.images.clear();
        if image_folder.exists() {
            for entry in WalkDir::new(image_folder)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                let path = entry.path();
                if let Ok(image) = image::open(path) {
                    let rgba = image.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                        let key = filename.to_lowercase();
                        let texture = ctx.load_texture(
                            &key,
                            egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_flat_samples().as_slice()),
                            egui::TextureOptions::LINEAR
                        );
                        self.images.insert(key, texture);
                    }
                }
            }
        }
    }
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: String::new(),
            game_folder: PathBuf::new(),
            save_folders: vec![PathBuf::new()],
            total_achievements: 0,
            unlocked_achievements: 0,
        }
    }
}

impl eframe::App for AchievementApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(receiver) = &self.receiver {
            if receiver.try_recv().is_ok() {
                self.load_earned_data();
                ctx.request_repaint();
            }
        }

        self.update_played_games();
        let total_score: usize = self.user_profile.played_games.iter()
            .map(|g| g.unlocked_achievements)
            .sum();

        let mut new_current_game = self.current_game;
        let mut games_to_remove = Vec::new();

        egui::TopBottomPanel::top("main_header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("🏠 Home").clicked() {
                    self.current_view = CurrentView::Home;
                }

                ui.horizontal(|ui| {
                    ScrollArea::horizontal().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (i, game) in self.games.iter().enumerate() {
                                let game_name = &game.name;
                                let selected = new_current_game == Some(i);
                                let progress = format!(" ({}/{})", game.unlocked_achievements, game.total_achievements);

                                ui.horizontal(|ui| {
                                    if ui.selectable_label(selected, format!("{}{}", game_name, progress)).clicked() {
                                        new_current_game = Some(i);
                                        self.current_view = CurrentView::Game;
                                    }

                                    if ui.button("❌").on_hover_text("Remove game").clicked() {
                                        games_to_remove.push(i);
                                    }
                                });
                            }
                        });
                    });
                    
                    if ui.button("+").on_hover_text("Add new game").clicked() {
                        self.show_config_window = true;
                    }
                    
                    if self.current_view == CurrentView::Game && self.current_game.is_some() {
                        if ui.button("⟳ Force Refresh").clicked() {
                            self.load_game_data(ctx);
                        }
                    }
                });
            });
        });

        // Handle game changes after UI rendering
        if new_current_game != self.current_game {
            self.current_game = new_current_game;
            if self.current_game.is_some() {
                self.load_game_data(ctx);
            }
        }

        // Handle game removals
        for &index in games_to_remove.iter().rev() {
            self.remove_game(index);
            if self.current_game == Some(index) {
                self.current_game = None;
            }
        }

        match self.current_view {
            CurrentView::Home => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            if self.editing_username {
                                if ui.text_edit_singleline(&mut self.user_profile.username).lost_focus() {
                                    self.editing_username = false;
                                    self.save_profile();
                                    self.update_username_configs();
                                }
                                if ui.button("💾").clicked() {
                                    self.editing_username = false;
                                    self.save_profile();
                                    self.update_username_configs();
                                }
                            } else {
                                ui.label(format!("Username: {}", self.user_profile.username));
                                if ui.button("✏️").clicked() {
                                    self.editing_username = true;
                                }
                            }

                            ui.separator();
                            if self.editing_id {
                                let response = ui.text_edit_singleline(&mut self.user_profile.user_id);
                                if response.lost_focus() || ui.button("💾").clicked() {
                                    if is_valid_id(&self.user_profile.user_id) {
                                        self.editing_id = false;
                                        self.save_profile();
                                        self.update_username_configs();
                                    } else {
                                        self.error_message = Some("ID must be 17 numeric characters".to_string());
                                    }
                                }
                                if ui.button("🎲").on_hover_text("Generate new ID").clicked() {
                                    self.user_profile.user_id = generate_random_id();
                                }
                            } else {
                                ui.label(format!("ID: {}", self.user_profile.user_id));
                                if ui.button("✏️").on_hover_text("Edit ID").clicked() {
                                    self.editing_id = true;
                                }
                            }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.heading(format!("Total Score: {}", total_score));
                            });
                        });

                        ui.separator();

                        ScrollArea::vertical().show(ui, |ui| {
                            let mut played_games = self.user_profile.played_games.clone();
                            played_games.sort_by(|a, b| a.name.cmp(&b.name));

                            for game in played_games {
                                ui.vertical(|ui| {
                                    let progress = if game.total_achievements > 0 {
                                        game.unlocked_achievements as f32 / game.total_achievements as f32
                                    } else {
                                        0.0
                                    };

                                    ui.horizontal(|ui| {
                                        if game.active {
                                            if ui.button(&game.name).clicked() {
                                                if let Some(index) = self.games.iter().position(|g| g.id == game.id) {
                                                    self.current_view = CurrentView::Game;
                                                    self.current_game = Some(index);
                                                    self.load_game_data(ctx);
                                                }
                                            }
                                        } else {
                                            ui.label(&game.name);
                                        }

                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(format!("{}/{} Achievements", 
                                                game.unlocked_achievements, 
                                                game.total_achievements
                                            ));
                                        });
                                    });

                                    ui.horizontal(|ui| {
                                        ui.add(egui::ProgressBar::new(progress).show_percentage());
                                    });

                                    ui.separator();
                                });
                            }
                        });
                    });
                });
            },
            CurrentView::Game => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(err) = &self.error_message {
                        ui.colored_label(Color32::RED, err);
                    }

                    if self.current_game.is_some() {
                        ui.vertical(|ui| {
                            if self.achievements.is_empty() {
                                ui.heading("No achievements loaded - check configuration");
                            } else {
                                let available_size = ui.available_size();
                                ScrollArea::vertical()
                                    .auto_shrink(false)
                                    .show(ui, |ui| {
                                        ui.set_width(available_size.x);
                                        egui::Frame::default()
                                            .inner_margin(egui::Margin::same(8))
                                            .show(ui, |ui| {
                                                let mut sorted_achievements: Vec<_> = self.achievements.values()
                                                    .map(|ach| (ach, self.earned.get(&ach.name).map(|ea| ea.earned).unwrap_or(false)))
                                                    .collect();

                                                sorted_achievements.sort_by(|(a, a_earned), (b, b_earned)| match (a_earned, b_earned) {
                                                    (true, true) => self.earned.get(&b.name).unwrap().earned_time.cmp(&self.earned.get(&a.name).unwrap().earned_time),
                                                    (true, false) => std::cmp::Ordering::Less,
                                                    (false, true) => std::cmp::Ordering::Greater,
                                                    (false, false) => a.display_name.cmp(&b.display_name),
                                                });

                                                for (i, (ach, earned)) in sorted_achievements.iter().enumerate() {
                                                    if ach.hidden == 1 && !earned { continue; }

                                                    ui.vertical(|ui| {
                                                        ui.add_space(8.0);
                                                        self.display_achievement(ui, ach);
                                                        ui.add_space(8.0);
                                                    });

                                                    if i < sorted_achievements.len() - 1 {
                                                        ui.separator();
                                                    }
                                                }
                                            });
                                    });
                            }
                        });
                    }
                });
            }
        }

        if self.show_config_window {
            let mut should_close = false;
            let mut should_save = false;
            let mut window_open = self.show_config_window;

            egui::Window::new("Game Configuration")
                .open(&mut window_open)
                .show(ctx, |ui| {
                    ui.label("Game Name:");
                    let name_response = ui.text_edit_singleline(&mut self.new_game.name);

                    if self.need_focus {
                        ui.memory_mut(|mem| mem.request_focus(name_response.id));
                        self.need_focus = false;
                    }

                    ui.label("Game ID:");
                    ui.text_edit_singleline(&mut self.new_game.id);

                    ui.label("Game Folder (should contain steam_settings/ or achievements.json):");
                    ui.horizontal(|ui| {
                        let mut path_str = self.new_game.game_folder.to_string_lossy().into_owned();
                        if ui.text_edit_singleline(&mut path_str).changed() {
                            self.new_game.game_folder = PathBuf::from(path_str);
                        }
                        if ui.button("📁").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                self.new_game.game_folder = path;
                            }
                        }
                    });

                    ui.label("Save Folders (where to look for GSE Saves):");
                    let mut to_remove = None;
                    for (i, folder) in self.new_game.save_folders.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            let mut path_str = folder.to_string_lossy().into_owned();
                            if ui.text_edit_singleline(&mut path_str).changed() {
                                *folder = PathBuf::from(path_str);
                            }
                            if ui.button("📁").clicked() {
                                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                    *folder = path;
                                }
                            }
                            if ui.button("❌").clicked() {
                                to_remove = Some(i);
                            }
                        });
                    }
                    
                    if let Some(i) = to_remove {
                        self.new_game.save_folders.remove(i);
                    }
                    
                    if ui.button("➕ Add Save Folder").clicked() {
                        self.new_game.save_folders.push(PathBuf::new());
                    }

                    ui.separator();
                    
                    ui.horizontal(|ui| {
                        if ui.button("🚫 Cancel").clicked() {
                            should_close = true;
                        }
                        
                        let button_text = if self.editing_game.is_some() {
                            "💾 Save Changes"
                        } else {
                            "✅ Add Game"
                        };
                        
                        if ui.button(button_text).clicked() {
                            match AchievementApp::validate_game_config(&self.new_game) {
                                Ok(_) => should_save = true,
                                Err(e) => self.error_message = Some(e),
                            }
                        }
                    });
                    
                    if let Some(err) = &self.error_message {
                        ui.colored_label(Color32::RED, err);
                    }
                });

            self.show_config_window = window_open;

            if should_save {
                let new_game = self.new_game.clone();
                if let Some(index) = self.editing_game {
                    self.games[index] = new_game;
                } else {
                    self.games.push(new_game);
                }
                self.save_config();
                self.setup_watchers();
                self.current_game = Some(self.games.len() - 1);
                self.load_game_data(ctx);
                should_close = true;
                self.update_username_configs();
            }

            if should_close {
                self.show_config_window = false;
                self.editing_game = None;
                self.new_game = GameConfig::default();
                self.error_message = None;
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_config();
        self.save_profile();
    }
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Achievement Viewer",
        native_options,
        Box::new(|_cc| {
            let mut app = AchievementApp {
                games: AchievementApp::load_config(),
                current_game: None,
                current_view: CurrentView::Home,
                new_game: GameConfig::default(),
                achievements: HashMap::new(),
                earned: HashMap::new(),
                images: HashMap::new(),
                watchers: Vec::new(),
                show_config_window: false,
                user_profile: AchievementApp::load_profile(),
                editing_username: false,
                editing_id: false,
                #[cfg(not(target_arch = "wasm32"))]
                receiver: None,
                error_message: None,
                editing_game: None,
                need_focus: true,
            };

            #[cfg(not(target_arch = "wasm32"))]
            app.setup_watchers();
            app.update_username_configs();

            Ok(Box::new(app))
        }),
    )
}

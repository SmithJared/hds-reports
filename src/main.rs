use anyhow::Result;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct Row {
    #[serde(rename = "Category")]
    category: String,

    #[serde(rename = "Name")]
    name: String,

    #[serde(rename = "ASIN")]
    asin: String,

    #[serde(rename = "Tracking ID")]
    tracking_id: String,

    #[serde(rename = "Revenue($)")]
    revenue: String,

    #[serde(rename = "Ad Fees($)")]
    ad_fee: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ReportType {
    Daily,
    Weekly,
}

impl ReportType {
    fn top_n(&self) -> usize {
        match self {
            ReportType::Daily => 3,
            ReportType::Weekly => 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Group {
    id: usize,
    name: String,
    tracking_ids: HashSet<String>,
}

#[derive(Debug, Clone)]
struct ProductStats {
    asin: String,
    name: String,
    total_ad_fee: f64,
    total_revenue: f64,
}

#[derive(Debug, Clone)]
struct CategoryStats {
    category: String,
    total_ad_fee: f64,
    total_revenue: f64,
}

type ProductMap = HashMap<String, ProductStats>;
type TrackingMap = HashMap<String, ProductMap>;
type CategoryMap = HashMap<String, CategoryStats>;

fn parse_money(raw: &str) -> f64 {
    raw.replace(['$', ','], "").parse::<f64>().unwrap_or(0.0)
}

fn parse_ad_fee(raw: &str) -> f64 {
    raw.replace(['$', ','], "").parse::<f64>().unwrap_or(0.0)
}

fn update_stats(map: &mut ProductMap, asin: &str, name: &str, ad_fee: f64, revenue: f64) {
    let entry = map.entry(asin.to_string()).or_insert_with(|| ProductStats {
        asin: asin.to_string(),
        name: name.to_string(),
        total_ad_fee: 0.0,
        total_revenue: 0.0,
    });

    entry.total_ad_fee += ad_fee;
    entry.total_revenue += revenue;
}

#[derive(Clone)]
struct ReportResults {
    report_type: ReportType,
    top_overall_ad_fee: Option<ProductStats>,
    top_overall_revenue: Option<ProductStats>,
    by_tracking_id_ad_fee: Vec<(String, Vec<ProductStats>)>,
    by_tracking_id_revenue: Vec<(String, Vec<ProductStats>)>,
    best_category_by_ad_fee: Vec<(String, Option<CategoryStats>)>,
    best_category_by_revenue: Vec<(String, Option<CategoryStats>)>,
}

fn process_csv(path: &str, report_type: ReportType) -> Result<ReportResults> {
    if !std::path::Path::new(path).exists() {
        anyhow::bail!("CSV file does not exist: {}", path);
    }

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    // Skip the first line (title)
    let mut first_line = String::new();
    reader.read_line(&mut first_line)?;

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(reader);

    // Skip the title row
    let mut dummy = csv::StringRecord::new();
    rdr.read_record(&mut dummy)?;

    let mut overall_by_asin: ProductMap = HashMap::new();
    let mut by_tracking_id: TrackingMap = HashMap::new();
    let mut categories_by_tracking_id: HashMap<String, CategoryMap> = HashMap::new();

    for result in rdr.deserialize::<Row>() {
        let row = result?;

        let asin = row.asin.trim();
        if asin.is_empty() {
            continue;
        }

        let ad_fee = parse_ad_fee(&row.ad_fee);
        let revenue = parse_money(&row.revenue);

        // Overall aggregation
        update_stats(&mut overall_by_asin, asin, &row.name, ad_fee, revenue);

        // Per Tracking ID aggregation
        let tracking_id = if row.tracking_id.trim().is_empty() {
            "UNKNOWN"
        } else {
            row.tracking_id.trim()
        };

        let product_map = by_tracking_id
            .entry(tracking_id.to_string())
            .or_insert_with(HashMap::new);

        update_stats(product_map, asin, &row.name, ad_fee, revenue);

        // Track categories per tracking ID
        let category = if row.category.trim().is_empty() {
            "UNKNOWN"
        } else {
            row.category.trim()
        };

        let category_map = categories_by_tracking_id
            .entry(tracking_id.to_string())
            .or_insert_with(HashMap::new);

        let category_entry = category_map
            .entry(category.to_string())
            .or_insert_with(|| CategoryStats {
                category: category.to_string(),
                total_ad_fee: 0.0,
                total_revenue: 0.0,
            });

        category_entry.total_ad_fee += ad_fee;
        category_entry.total_revenue += revenue;
    }

    let top_n = report_type.top_n();

    // Get top overall products by ad fee
    let top_overall_ad_fee = overall_by_asin
        .values()
        .max_by(|a, b| a.total_ad_fee.partial_cmp(&b.total_ad_fee).unwrap())
        .cloned();

    // Get top overall products by revenue
    let top_overall_revenue = overall_by_asin
        .values()
        .max_by(|a, b| a.total_revenue.partial_cmp(&b.total_revenue).unwrap())
        .cloned();

    // Get top N per tracking ID by ad fee
    let mut by_tracking_id_ad_fee: Vec<(String, Vec<ProductStats>)> = Vec::new();
    for (tracking_id, products) in &by_tracking_id {
        let mut top: Vec<_> = products.values().cloned().collect();
        top.sort_by(|a, b| b.total_ad_fee.partial_cmp(&a.total_ad_fee).unwrap());
        top.truncate(top_n);
        by_tracking_id_ad_fee.push((tracking_id.clone(), top));
    }

    // Get top N per tracking ID by revenue
    let mut by_tracking_id_revenue: Vec<(String, Vec<ProductStats>)> = Vec::new();
    for (tracking_id, products) in by_tracking_id {
        let mut top: Vec<_> = products.values().cloned().collect();
        top.sort_by(|a, b| b.total_revenue.partial_cmp(&a.total_revenue).unwrap());
        top.truncate(top_n);
        by_tracking_id_revenue.push((tracking_id, top));
    }

    // Sort by tracking ID for consistent display
    by_tracking_id_ad_fee.sort_by(|a, b| a.0.cmp(&b.0));
    by_tracking_id_revenue.sort_by(|a, b| a.0.cmp(&b.0));

    // Find best performing category per tracking ID
    let mut best_category_by_ad_fee: Vec<(String, Option<CategoryStats>)> = Vec::new();
    let mut best_category_by_revenue: Vec<(String, Option<CategoryStats>)> = Vec::new();

    for (tracking_id, category_map) in categories_by_tracking_id {
        let best_by_fee = category_map
            .values()
            .max_by(|a, b| a.total_ad_fee.partial_cmp(&b.total_ad_fee).unwrap())
            .cloned();

        let best_by_rev = category_map
            .values()
            .max_by(|a, b| a.total_revenue.partial_cmp(&b.total_revenue).unwrap())
            .cloned();

        best_category_by_ad_fee.push((tracking_id.clone(), best_by_fee));
        best_category_by_revenue.push((tracking_id, best_by_rev));
    }

    // Sort by tracking ID for consistent display
    best_category_by_ad_fee.sort_by(|a, b| a.0.cmp(&b.0));
    best_category_by_revenue.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(ReportResults {
        report_type,
        top_overall_ad_fee,
        top_overall_revenue,
        by_tracking_id_ad_fee,
        by_tracking_id_revenue,
        best_category_by_ad_fee,
        best_category_by_revenue,
    })
}

const GROUPS_FILENAME: &str = "tracking_groups.json";

fn get_groups_file_path() -> PathBuf {
    // Try to get the executable's directory, fall back to current directory if that fails
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            return exe_dir.join(GROUPS_FILENAME);
        }
    }
    // Fallback to current directory
    PathBuf::from(GROUPS_FILENAME)
}

#[derive(Clone)]
struct GroupedResults {
    top_products_by_ad_fee: Vec<ProductStats>,
    top_products_by_revenue: Vec<ProductStats>,
    best_category_by_ad_fee: Option<CategoryStats>,
    best_category_by_revenue: Option<CategoryStats>,
}

fn aggregate_tracking_ids(
    tracking_ids: &HashSet<String>,
    results: &ReportResults,
) -> GroupedResults {
    let top_n = results.report_type.top_n();

    // Aggregate products across all tracking IDs in the group
    let mut combined_products: HashMap<String, ProductStats> = HashMap::new();

    for tracking_id in tracking_ids {
        // Add products from ad_fee list
        if let Some((_, products)) = results
            .by_tracking_id_ad_fee
            .iter()
            .find(|(id, _)| id == tracking_id)
        {
            for product in products {
                let entry = combined_products
                    .entry(product.asin.clone())
                    .or_insert_with(|| ProductStats {
                        asin: product.asin.clone(),
                        name: product.name.clone(),
                        total_ad_fee: 0.0,
                        total_revenue: 0.0,
                    });
                entry.total_ad_fee += product.total_ad_fee;
            }
        }

        // Add products from revenue list
        if let Some((_, products)) = results
            .by_tracking_id_revenue
            .iter()
            .find(|(id, _)| id == tracking_id)
        {
            for product in products {
                let entry = combined_products
                    .entry(product.asin.clone())
                    .or_insert_with(|| ProductStats {
                        asin: product.asin.clone(),
                        name: product.name.clone(),
                        total_ad_fee: 0.0,
                        total_revenue: 0.0,
                    });
                entry.total_revenue += product.total_revenue;
            }
        }
    }

    // Get top N by ad fee
    let mut top_by_ad_fee: Vec<_> = combined_products.values().cloned().collect();
    top_by_ad_fee.sort_by(|a, b| b.total_ad_fee.partial_cmp(&a.total_ad_fee).unwrap());
    top_by_ad_fee.truncate(top_n);

    // Get top N by revenue
    let mut top_by_revenue: Vec<_> = combined_products.values().cloned().collect();
    top_by_revenue.sort_by(|a, b| b.total_revenue.partial_cmp(&a.total_revenue).unwrap());
    top_by_revenue.truncate(top_n);

    // Aggregate categories
    let mut combined_categories: HashMap<String, CategoryStats> = HashMap::new();

    for tracking_id in tracking_ids {
        // Add category from ad_fee list
        if let Some((_, cat_opt)) = results
            .best_category_by_ad_fee
            .iter()
            .find(|(id, _)| id == tracking_id)
        {
            if let Some(cat) = cat_opt {
                let entry = combined_categories
                    .entry(cat.category.clone())
                    .or_insert_with(|| CategoryStats {
                        category: cat.category.clone(),
                        total_ad_fee: 0.0,
                        total_revenue: 0.0,
                    });
                entry.total_ad_fee += cat.total_ad_fee;
            }
        }

        // Add category from revenue list
        if let Some((_, cat_opt)) = results
            .best_category_by_revenue
            .iter()
            .find(|(id, _)| id == tracking_id)
        {
            if let Some(cat) = cat_opt {
                let entry = combined_categories
                    .entry(cat.category.clone())
                    .or_insert_with(|| CategoryStats {
                        category: cat.category.clone(),
                        total_ad_fee: 0.0,
                        total_revenue: 0.0,
                    });
                entry.total_revenue += cat.total_revenue;
            }
        }
    }

    let best_cat_by_fee = combined_categories
        .values()
        .max_by(|a, b| a.total_ad_fee.partial_cmp(&b.total_ad_fee).unwrap())
        .cloned();

    let best_cat_by_revenue = combined_categories
        .values()
        .max_by(|a, b| a.total_revenue.partial_cmp(&b.total_revenue).unwrap())
        .cloned();

    GroupedResults {
        top_products_by_ad_fee: top_by_ad_fee,
        top_products_by_revenue: top_by_revenue,
        best_category_by_ad_fee: best_cat_by_fee,
        best_category_by_revenue: best_cat_by_revenue,
    }
}

fn save_groups(groups: &[Group]) -> Result<()> {
    let path = get_groups_file_path();
    let json = serde_json::to_string_pretty(groups)?;
    let mut file = File::create(path)?;
    file.write_all(json.as_bytes())?;
    Ok(())
}

fn load_groups() -> Vec<Group> {
    let path = get_groups_file_path();
    if let Ok(file) = File::open(path) {
        if let Ok(groups) = serde_json::from_reader(file) {
            return groups;
        }
    }
    Vec::new()
}

struct ReportApp {
    selected_file: Option<String>,
    results: Option<ReportResults>,
    error_message: Option<String>,
    selected_tracking_ids: HashSet<String>,
    report_type: ReportType,
    groups: Vec<Group>,
    next_group_id: usize,
    selected_groups: HashSet<usize>,
    editing_group_id: Option<usize>,
    editing_group_name: String,
    editing_tracking_ids_for_group: Option<usize>,
    new_group_name: String,
}

impl Default for ReportApp {
    fn default() -> Self {
        let groups = load_groups();
        let next_group_id = groups.iter().map(|g| g.id).max().unwrap_or(0) + 1;

        Self {
            selected_file: None,
            results: None,
            error_message: None,
            selected_tracking_ids: HashSet::new(),
            report_type: ReportType::Daily,
            groups,
            next_group_id,
            selected_groups: HashSet::new(),
            editing_group_id: None,
            editing_group_name: String::new(),
            editing_tracking_ids_for_group: None,
            new_group_name: String::new(),
        }
    }
}

impl eframe::App for ReportApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Affiliate Report Analyzer");
            ui.add_space(10.0);

            // Report type selector
            ui.horizontal(|ui| {
                ui.label("Report Type:");
                ui.radio_value(&mut self.report_type, ReportType::Daily, "Daily (Top 3)");
                ui.radio_value(&mut self.report_type, ReportType::Weekly, "Weekly (Top 10)");
            });

            ui.add_space(10.0);

            // File selection button
            if ui.button("Select CSV File").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("CSV", &["csv"])
                    .pick_file()
                {
                    let path_str = path.to_string_lossy().to_string();
                    self.selected_file = Some(path_str.clone());

                    // Process the file
                    match process_csv(&path_str, self.report_type) {
                        Ok(results) => {
                            self.results = Some(results);
                            self.error_message = None;
                            self.selected_tracking_ids.clear();
                        }
                        Err(e) => {
                            self.error_message = Some(format!("Error: {}", e));
                            self.results = None;
                            self.selected_tracking_ids.clear();
                        }
                    }
                }
            }

            // Display selected file
            if let Some(path) = &self.selected_file {
                ui.add_space(5.0);
                ui.label(format!("Selected: {}", path));
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            // Display error if any
            if let Some(error) = &self.error_message {
                ui.colored_label(egui::Color32::RED, error);
            }

            // Display results
            if let Some(results) = &self.results {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let top_n = results.report_type.top_n();

                    // Top overall products
                    ui.heading("Top Performing Products Overall");
                    ui.add_space(5.0);

                    let available_width = ui.available_width();
                    let column_width = (available_width - 20.0) / 2.0;

                    ui.horizontal(|ui| {
                        // Ad Fee column
                        ui.vertical(|ui| {
                            ui.set_width(column_width);
                            ui.strong("By Commission:");
                            if let Some(p) = &results.top_overall_ad_fee {
                                ui.label(format!("ASIN: {}", p.asin));
                                ui.label("Name:");
                                ui.label(&p.name);
                                let url = format!("https://www.amazon.com/gp/product/{}", p.asin);
                                ui.hyperlink(&url);
                                ui.label(format!("Commission: ${:.2}", p.total_ad_fee));
                            } else {
                                ui.label("No products found");
                            }
                        });

                        ui.add_space(10.0);

                        // Revenue column
                        ui.vertical(|ui| {
                            ui.set_width(column_width);
                            ui.strong("By Revenue:");
                            if let Some(p) = &results.top_overall_revenue {
                                ui.label(format!("ASIN: {}", p.asin));
                                ui.label("Name:");
                                ui.label(&p.name);
                                let url = format!("https://www.amazon.com/gp/product/{}", p.asin);
                                ui.hyperlink(&url);
                                ui.label(format!("Revenue: ${:.2}", p.total_revenue));
                            } else {
                                ui.label("No products found");
                            }
                        });
                    });

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Tracking ID selection
                    ui.heading("Select Tracking IDs to Display");
                    ui.add_space(5.0);

                    // Select All / Deselect All buttons
                    ui.horizontal(|ui| {
                        if ui.button("Select All").clicked() {
                            for (tracking_id, _) in &results.by_tracking_id_ad_fee {
                                self.selected_tracking_ids.insert(tracking_id.clone());
                            }
                        }
                        if ui.button("Deselect All").clicked() {
                            self.selected_tracking_ids.clear();
                        }
                    });

                    ui.add_space(10.0);

                    // Create group section
                    ui.horizontal(|ui| {
                        ui.label("Group name:");
                        ui.text_edit_singleline(&mut self.new_group_name);

                        if ui.button("Create Group from Selected").clicked() {
                            if !self.selected_tracking_ids.is_empty() {
                                let group_name = if self.new_group_name.trim().is_empty() {
                                    format!("Group {}", self.next_group_id)
                                } else {
                                    self.new_group_name.trim().to_string()
                                };

                                let group = Group {
                                    id: self.next_group_id,
                                    name: group_name,
                                    tracking_ids: self.selected_tracking_ids.clone(),
                                };
                                self.groups.push(group);
                                self.next_group_id += 1;
                                if let Err(e) = save_groups(&self.groups) {
                                    self.error_message = Some(format!("Failed to save groups: {}", e));
                                }
                                self.selected_tracking_ids.clear();
                                self.new_group_name.clear();
                            }
                        }
                    });

                    ui.add_space(10.0);

                    // Checkboxes for each tracking ID in a grid layout
                    ui.group(|ui| {
                        egui::Grid::new("tracking_id_grid")
                            .num_columns(3)
                            .spacing([20.0, 10.0])
                            .striped(false)
                            .show(ui, |ui| {
                                let mut col_count = 0;
                                for (tracking_id, _) in &results.by_tracking_id_ad_fee {
                                    let mut is_selected = self.selected_tracking_ids.contains(tracking_id);
                                    if ui.checkbox(&mut is_selected, tracking_id).changed() {
                                        if is_selected {
                                            self.selected_tracking_ids.insert(tracking_id.clone());
                                        } else {
                                            self.selected_tracking_ids.remove(tracking_id);
                                        }
                                    }

                                    col_count += 1;
                                    if col_count % 3 == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                    });

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Display groups
                    if !self.groups.is_empty() {
                        ui.heading("Saved Groups");
                        ui.add_space(5.0);

                        let mut groups_to_delete = Vec::new();
                        let mut groups_to_update = Vec::new();
                        let mut rename_updates = Vec::new();

                        for group in &self.groups {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    // Checkbox to select group
                                    let mut is_selected = self.selected_groups.contains(&group.id);
                                    if ui.checkbox(&mut is_selected, "").changed() {
                                        if is_selected {
                                            self.selected_groups.insert(group.id);
                                        } else {
                                            self.selected_groups.remove(&group.id);
                                        }
                                    }

                                    // Group name (editable if in edit mode)
                                    if self.editing_group_id == Some(group.id) {
                                        ui.text_edit_singleline(&mut self.editing_group_name);
                                        if ui.button("Save").clicked() {
                                            rename_updates.push((group.id, self.editing_group_name.clone()));
                                            self.editing_group_id = None;
                                        }
                                        if ui.button("Cancel").clicked() {
                                            self.editing_group_id = None;
                                        }
                                    } else {
                                        ui.strong(&group.name);
                                        ui.label(format!("({} tracking IDs)", group.tracking_ids.len()));

                                        if ui.button("Rename").clicked() {
                                            self.editing_group_id = Some(group.id);
                                            self.editing_group_name = group.name.clone();
                                        }

                                        if ui.button("Edit Tracking IDs").clicked() {
                                            self.editing_tracking_ids_for_group = Some(group.id);
                                        }

                                        if ui.button("Delete").clicked() {
                                            groups_to_delete.push(group.id);
                                        }
                                    }
                                });

                                // Show tracking IDs in the group
                                ui.label(format!("Tracking IDs: {}", group.tracking_ids.iter().cloned().collect::<Vec<_>>().join(", ")));

                                // Edit tracking IDs mode
                                if self.editing_tracking_ids_for_group == Some(group.id) {
                                    ui.add_space(5.0);
                                    ui.label("Select tracking IDs to include in this group:");

                                    let mut updated_tracking_ids = group.tracking_ids.clone();

                                    egui::Grid::new(format!("edit_tracking_grid_{}", group.id))
                                        .num_columns(3)
                                        .spacing([20.0, 10.0])
                                        .show(ui, |ui| {
                                            let mut col_count = 0;
                                            for (tracking_id, _) in &results.by_tracking_id_ad_fee {
                                                let mut is_in_group = updated_tracking_ids.contains(tracking_id);
                                                if ui.checkbox(&mut is_in_group, tracking_id).changed() {
                                                    if is_in_group {
                                                        updated_tracking_ids.insert(tracking_id.clone());
                                                    } else {
                                                        updated_tracking_ids.remove(tracking_id);
                                                    }
                                                }
                                                col_count += 1;
                                                if col_count % 3 == 0 {
                                                    ui.end_row();
                                                }
                                            }
                                        });

                                    ui.add_space(5.0);
                                    if ui.button("Done Editing").clicked() {
                                        groups_to_update.push((group.id, updated_tracking_ids));
                                        self.editing_tracking_ids_for_group = None;
                                    }
                                }
                            });
                            ui.add_space(5.0);
                        }

                        // Check if any changes need to be saved
                        let has_changes = !rename_updates.is_empty() || !groups_to_update.is_empty() || !groups_to_delete.is_empty();

                        // Apply rename updates
                        for (id, new_name) in rename_updates {
                            if let Some(g) = self.groups.iter_mut().find(|g| g.id == id) {
                                g.name = new_name;
                            }
                        }

                        // Apply tracking ID updates
                        for (id, new_tracking_ids) in groups_to_update {
                            if let Some(g) = self.groups.iter_mut().find(|g| g.id == id) {
                                g.tracking_ids = new_tracking_ids;
                            }
                        }

                        // Delete marked groups
                        for id in groups_to_delete {
                            self.groups.retain(|g| g.id != id);
                            self.selected_groups.remove(&id);
                        }

                        // Save if any changes were made
                        if has_changes {
                            if let Err(e) = save_groups(&self.groups) {
                                self.error_message = Some(format!("Failed to save groups: {}", e));
                            }
                        }

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);
                    }

                    // Display selected tracking IDs
                    if !self.selected_tracking_ids.is_empty() {
                        ui.heading(format!("Top {} Products Per Selected Tracking ID", top_n));
                        ui.add_space(10.0);

                        for (tracking_id, _) in &results.by_tracking_id_ad_fee {
                            if !self.selected_tracking_ids.contains(tracking_id) {
                                continue;
                            }

                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    ui.strong(format!("Tracking ID: {}", tracking_id));

                                    if ui.button("📋 Copy").clicked() {
                                        let mut copy_text = format!("Tracking ID: {}\n\n", tracking_id);

                                        // Add category info
                                        if let Some((_, cat_opt)) = results.best_category_by_ad_fee.iter()
                                            .find(|(id, _)| id == tracking_id)
                                        {
                                            if let Some(cat) = cat_opt {
                                                copy_text.push_str(&format!("Best Category by Commission: {} (${:.2})\n", cat.category, cat.total_ad_fee));
                                            }
                                        }
                                        if let Some((_, cat_opt)) = results.best_category_by_revenue.iter()
                                            .find(|(id, _)| id == tracking_id)
                                        {
                                            if let Some(cat) = cat_opt {
                                                copy_text.push_str(&format!("Best Category by Revenue: {} (${:.2})\n", cat.category, cat.total_revenue));
                                            }
                                        }
                                        copy_text.push_str("\n");

                                        // Add top products by commission
                                        copy_text.push_str(&format!("Top {} by Commission:\n", top_n));
                                        if let Some(products) = results.by_tracking_id_ad_fee.iter()
                                            .find(|(id, _)| id == tracking_id)
                                            .map(|(_, prods)| prods)
                                        {
                                            for (i, p) in products.iter().enumerate() {
                                                copy_text.push_str(&format!("{}. {}\n   {}\n   https://www.amazon.com/gp/product/{}\n   Commission: ${:.2}\n\n",
                                                    i + 1, p.asin, p.name, p.asin, p.total_ad_fee));
                                            }
                                        }

                                        // Add top products by revenue
                                        copy_text.push_str(&format!("Top {} by Revenue:\n", top_n));
                                        if let Some(products) = results.by_tracking_id_revenue.iter()
                                            .find(|(id, _)| id == tracking_id)
                                            .map(|(_, prods)| prods)
                                        {
                                            for (i, p) in products.iter().enumerate() {
                                                copy_text.push_str(&format!("{}. {}\n   {}\n   https://www.amazon.com/gp/product/{}\n   Revenue: ${:.2}\n\n",
                                                    i + 1, p.asin, p.name, p.asin, p.total_revenue));
                                            }
                                        }

                                        ui.ctx().copy_text(copy_text);
                                    }
                                });
                                ui.add_space(3.0);

                                // Display best performing categories
                                ui.horizontal(|ui| {
                                    ui.label("Best Performing Category:");

                                    if let Some((_, cat_opt)) = results.best_category_by_ad_fee.iter()
                                        .find(|(id, _)| id == tracking_id)
                                    {
                                        if let Some(cat) = cat_opt {
                                            ui.label(format!("By Commission: {} (${:.2})", cat.category, cat.total_ad_fee));
                                        }
                                    }

                                    ui.add_space(10.0);

                                    if let Some((_, cat_opt)) = results.best_category_by_revenue.iter()
                                        .find(|(id, _)| id == tracking_id)
                                    {
                                        if let Some(cat) = cat_opt {
                                            ui.label(format!("By Revenue: {} (${:.2})", cat.category, cat.total_revenue));
                                        }
                                    }
                                });

                                ui.add_space(5.0);
                                ui.separator();
                                ui.add_space(5.0);

                                let available_width = ui.available_width();
                                let column_width = (available_width - 20.0) / 2.0;

                                ui.horizontal(|ui| {
                                    // Ad Fee products
                                    ui.vertical(|ui| {
                                        ui.set_width(column_width);
                                        ui.strong(format!("Top {} by Commission:", top_n));

                                        if let Some(products) = results.by_tracking_id_ad_fee.iter()
                                            .find(|(id, _)| id == tracking_id)
                                            .map(|(_, prods)| prods)
                                        {
                                            for (i, p) in products.iter().enumerate() {
                                                ui.label(format!("{}. {}", i + 1, p.asin));
                                                ui.label(format!("   {}", p.name));
                                                let url = format!("https://www.amazon.com/gp/product/{}", p.asin);
                                                ui.horizontal(|ui| {
                                                    ui.label("   ");
                                                    ui.hyperlink(&url);
                                                });
                                                ui.label(format!("   Commission: ${:.2}", p.total_ad_fee));
                                                if i < products.len() - 1 {
                                                    ui.add_space(2.0);
                                                }
                                            }
                                        }
                                    });

                                    ui.add_space(10.0);

                                    // Revenue products
                                    ui.vertical(|ui| {
                                        ui.set_width(column_width);
                                        ui.strong(format!("Top {} by Revenue:", top_n));

                                        if let Some(products) = results.by_tracking_id_revenue.iter()
                                            .find(|(id, _)| id == tracking_id)
                                            .map(|(_, prods)| prods)
                                        {
                                            for (i, p) in products.iter().enumerate() {
                                                ui.label(format!("{}. {}", i + 1, p.asin));
                                                ui.label(format!("   {}", p.name));
                                                let url = format!("https://www.amazon.com/gp/product/{}", p.asin);
                                                ui.horizontal(|ui| {
                                                    ui.label("   ");
                                                    ui.hyperlink(&url);
                                                });
                                                ui.label(format!("   Rev: ${:.2}", p.total_revenue));
                                                if i < products.len() - 1 {
                                                    ui.add_space(2.0);
                                                }
                                            }
                                        }
                                    });
                                });
                            });
                            ui.add_space(5.0);
                        }
                    } else {
                        ui.label("No tracking IDs selected. Select tracking IDs above to view their data.");
                    }

                    // Display selected groups
                    if !self.selected_groups.is_empty() {
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);

                        ui.heading(format!("Top {} Products Per Selected Group", top_n));
                        ui.add_space(10.0);

                        for group_id in &self.selected_groups {
                            if let Some(group) = self.groups.iter().find(|g| g.id == *group_id) {
                                let grouped_results = aggregate_tracking_ids(&group.tracking_ids, results);

                                ui.group(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.strong(format!("Group: {}", group.name));

                                        if ui.button("📋 Copy").clicked() {
                                            let mut copy_text = format!("Group: {}\n", group.name);
                                            copy_text.push_str(&format!("Tracking IDs: {}\n\n", group.tracking_ids.iter().cloned().collect::<Vec<_>>().join(", ")));

                                            // Add category info
                                            if let Some(cat) = &grouped_results.best_category_by_ad_fee {
                                                copy_text.push_str(&format!("Best Category by Commission: {} (${:.2})\n", cat.category, cat.total_ad_fee));
                                            }
                                            if let Some(cat) = &grouped_results.best_category_by_revenue {
                                                copy_text.push_str(&format!("Best Category by Revenue: {} (${:.2})\n", cat.category, cat.total_revenue));
                                            }
                                            copy_text.push_str("\n");

                                            // Add top products by commission
                                            copy_text.push_str(&format!("Top {} by Commission:\n", top_n));
                                            for (i, p) in grouped_results.top_products_by_ad_fee.iter().enumerate() {
                                                copy_text.push_str(&format!("{}. {}\n   {}\n   https://www.amazon.com/gp/product/{}\n   Commission: ${:.2}\n\n",
                                                    i + 1, p.asin, p.name, p.asin, p.total_ad_fee));
                                            }

                                            // Add top products by revenue
                                            copy_text.push_str(&format!("Top {} by Revenue:\n", top_n));
                                            for (i, p) in grouped_results.top_products_by_revenue.iter().enumerate() {
                                                copy_text.push_str(&format!("{}. {}\n   {}\n   https://www.amazon.com/gp/product/{}\n   Revenue: ${:.2}\n\n",
                                                    i + 1, p.asin, p.name, p.asin, p.total_revenue));
                                            }

                                            ui.ctx().copy_text(copy_text);
                                        }
                                    });
                                    ui.label(format!("Tracking IDs: {}", group.tracking_ids.iter().cloned().collect::<Vec<_>>().join(", ")));
                                    ui.add_space(3.0);

                                    // Display best performing categories
                                    ui.horizontal(|ui| {
                                        ui.label("Best Performing Category:");

                                        if let Some(cat) = &grouped_results.best_category_by_ad_fee {
                                            ui.label(format!("By Commission: {} (${:.2})", cat.category, cat.total_ad_fee));
                                        }

                                        ui.add_space(10.0);

                                        if let Some(cat) = &grouped_results.best_category_by_revenue {
                                            ui.label(format!("By Revenue: {} (${:.2})", cat.category, cat.total_revenue));
                                        }
                                    });

                                    ui.add_space(5.0);
                                    ui.separator();
                                    ui.add_space(5.0);

                                    let available_width = ui.available_width();
                                    let column_width = (available_width - 20.0) / 2.0;

                                    ui.horizontal(|ui| {
                                        // Ad Fee products
                                        ui.vertical(|ui| {
                                            ui.set_width(column_width);
                                            ui.strong(format!("Top {} by Commission:", top_n));

                                            for (i, p) in grouped_results.top_products_by_ad_fee.iter().enumerate() {
                                                ui.label(format!("{}. {}", i + 1, p.asin));
                                                ui.label(format!("   {}", p.name));
                                                let url = format!("https://www.amazon.com/gp/product/{}", p.asin);
                                                ui.horizontal(|ui| {
                                                    ui.label("   ");
                                                    ui.hyperlink(&url);
                                                });
                                                ui.label(format!("   Commission: ${:.2}", p.total_ad_fee));
                                                if i < grouped_results.top_products_by_ad_fee.len() - 1 {
                                                    ui.add_space(2.0);
                                                }
                                            }
                                        });

                                        ui.add_space(10.0);

                                        // Revenue products
                                        ui.vertical(|ui| {
                                            ui.set_width(column_width);
                                            ui.strong(format!("Top {} by Revenue:", top_n));

                                            for (i, p) in grouped_results.top_products_by_revenue.iter().enumerate() {
                                                ui.label(format!("{}. {}", i + 1, p.asin));
                                                ui.label(format!("   {}", p.name));
                                                let url = format!("https://www.amazon.com/gp/product/{}", p.asin);
                                                ui.horizontal(|ui| {
                                                    ui.label("   ");
                                                    ui.hyperlink(&url);
                                                });
                                                ui.label(format!("   Rev: ${:.2}", p.total_revenue));
                                                if i < grouped_results.top_products_by_revenue.len() - 1 {
                                                    ui.add_space(2.0);
                                                }
                                            }
                                        });
                                    });
                                });
                                ui.add_space(5.0);
                            }
                        }
                    }
                });
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 700.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Affiliate Report Analyzer",
        options,
        Box::new(|_cc| Ok(Box::<ReportApp>::default())),
    )
}

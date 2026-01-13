use anyhow::Result;
use eframe::egui;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};

#[derive(Debug, Deserialize)]
struct Row {
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

#[derive(Debug, Clone)]
struct ProductStats {
    asin: String,
    name: String,
    total_ad_fee: f64,
    total_revenue: f64,
}

type ProductMap = HashMap<String, ProductStats>;
type TrackingMap = HashMap<String, ProductMap>;

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

    Ok(ReportResults {
        report_type,
        top_overall_ad_fee,
        top_overall_revenue,
        by_tracking_id_ad_fee,
        by_tracking_id_revenue,
    })
}

struct ReportApp {
    selected_file: Option<String>,
    results: Option<ReportResults>,
    error_message: Option<String>,
    selected_tracking_ids: HashSet<String>,
    report_type: ReportType,
}

impl Default for ReportApp {
    fn default() -> Self {
        Self {
            selected_file: None,
            results: None,
            error_message: None,
            selected_tracking_ids: HashSet::new(),
            report_type: ReportType::Daily,
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

                    ui.horizontal(|ui| {
                        // Ad Fee column
                        ui.vertical(|ui| {
                            ui.strong("By Commission:");
                            if let Some(p) = &results.top_overall_ad_fee {
                                ui.label(format!("ASIN: {}", p.asin));
                                ui.label(format!("Name: {}", p.name));
                                ui.label(format!("Commission: ${:.2}", p.total_ad_fee));
                            } else {
                                ui.label("No products found");
                            }
                        });

                        ui.add_space(40.0);

                        // Revenue column
                        ui.vertical(|ui| {
                            ui.strong("By Revenue:");
                            if let Some(p) = &results.top_overall_revenue {
                                ui.label(format!("ASIN: {}", p.asin));
                                ui.label(format!("Name: {}", p.name));
                                ui.label(format!("Revenue: ${:.2}", p.total_revenue));
                            } else {
                                ui.label("No products found");
                            }
                        });
                    });

                    ui.add_space(20.0);
                    ui.separator();
                    ui.add_space(20.0);

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

                    ui.add_space(20.0);
                    ui.separator();
                    ui.add_space(20.0);

                    // Display selected tracking IDs
                    if !self.selected_tracking_ids.is_empty() {
                        ui.heading(format!("Top {} Products Per Selected Tracking ID", top_n));
                        ui.add_space(10.0);

                        for (tracking_id, _) in &results.by_tracking_id_ad_fee {
                            if !self.selected_tracking_ids.contains(tracking_id) {
                                continue;
                            }

                            ui.group(|ui| {
                                ui.strong(format!("Tracking ID: {}", tracking_id));
                                ui.add_space(3.0);

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

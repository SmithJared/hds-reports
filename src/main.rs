use anyhow::Result;
use eframe::egui;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};

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

#[derive(Debug)]
struct CommissionRule {
    rate: f64,
    categories: &'static [&'static str],
}

const COMMISSION_RULES: &[CommissionRule] = &[
    CommissionRule {
        rate: 0.20,
        categories: &[
            "Amazon Fashion Private Brands",
            "Amazon Games",
            "Premium Beauty",
            "Clothing & Accessories",
            "Home",
            "Jewelry",
            "Kitchen & Dining",
            "Luggage",
            "Luxury Stores Beauty",
            "Luxury Stores Fashion",
            "Power & Hand Tools",
            "Shoes, Handbags, Wallets, Sunglasses",
            "Watches",
        ],
    },
    CommissionRule {
        rate: 0.10,
        categories: &["Beauty & Grooming"],
    },
    CommissionRule {
        rate: 0.08,
        categories: &[
            "CDs & Vinyl",
            "Digital Music",
            "Handmade",
            "Video On Demand: Rent or Buy",
        ],
    },
    CommissionRule {
        rate: 0.05,
        categories: &["Automotive", "Books & Textbooks"],
    },
    CommissionRule {
        rate: 0.045,
        categories: &[
            "Blink Devices",
            "Echo Devices",
            "Echo Look",
            "Fire TV Devices",
            "Fire TV Edition Smart TVs",
            "Fire Tablets",
            "Kindle E-readers",
            "Major Appliances",
            "Office & School Supplies",
            "Ring Accessories",
            "Ring Devices",
            "Sports & Fitness",
        ],
    },
    CommissionRule {
        rate: 0.04,
        categories: &["Fine Art"],
    },
    CommissionRule {
        rate: 0.04,
        categories: &[
            "Amazon Coins",
            "Baby & Nursery",
            "Business & Industrial Supplies",
            "Electronic Components & Home Audio",
            "Furniture",
            "Home Improvement",
            "Musical Instruments",
            "Outdoor Recreation",
            "Patio, Lawn & Garden",
            "Pet Food & Supplies",
            "Toys & Games",
        ],
    },
    CommissionRule {
        rate: 0.03,
        categories: &["Blu-Ray & DVD", "Computers, Tablets & Components"],
    },
    CommissionRule {
        rate: 0.025,
        categories: &["Home Entertainment: TV", "Video Game Downloads"],
    },
    CommissionRule {
        rate: 0.02,
        categories: &[
            "Amazon Fresh",
            "Grocery & Gourmet Food",
            "Health & Household",
            "Video Games",
        ],
    },
    CommissionRule {
        rate: 0.0,
        categories: &[
            "Amazon Gift Cards",
            "Appstore for Android",
            "Kindle Unlimited Memberships",
            "Other Gift Card Brands",
            "Pet Prescription Medications",
            "Prime Memberships",
            "Wine, Spirits & Beer",
        ],
    },
];

const DEFAULT_RATE: f64 = 0.04;

fn commission_rate(category: &str) -> f64 {
    let category = category.trim();

    for rule in COMMISSION_RULES {
        if rule.categories.iter().any(|c| *c == category) {
            return rule.rate;
        }
    }

    DEFAULT_RATE
}

#[derive(Debug, Clone)]
struct ProductStats {
    asin: String,
    name: String,
    total_profit: f64,
}

type ProductMap = HashMap<String, ProductStats>;
type TrackingMap = HashMap<String, ProductMap>;

fn parse_money(raw: &str) -> f64 {
    raw.replace(['$', ','], "").parse::<f64>().unwrap_or(0.0)
}

fn parse_ad_fee(raw: &str) -> f64 {
    raw.replace(['$', ','], "").parse::<f64>().unwrap_or(0.0)
}

fn update_stats(map: &mut ProductMap, asin: &str, name: &str, profit: f64) {
    let entry = map.entry(asin.to_string()).or_insert_with(|| ProductStats {
        asin: asin.to_string(),
        name: name.to_string(),
        total_profit: 0.0,
    });

    entry.total_profit += profit;
}

#[derive(Clone)]
struct ReportResults {
    top_overall: Option<ProductStats>,
    by_tracking_id: Vec<(String, Vec<ProductStats>)>,
}

fn process_csv(path: &str) -> Result<ReportResults> {
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

        let profit = parse_ad_fee(&row.ad_fee);

        // Overall aggregation
        update_stats(&mut overall_by_asin, asin, &row.name, profit);

        // Per Tracking ID aggregation
        let tracking_id = if row.tracking_id.trim().is_empty() {
            "UNKNOWN"
        } else {
            row.tracking_id.trim()
        };

        let product_map = by_tracking_id
            .entry(tracking_id.to_string())
            .or_insert_with(HashMap::new);

        update_stats(product_map, asin, &row.name, profit);
    }

    // Get top overall product
    let top_overall = overall_by_asin
        .values()
        .max_by(|a, b| a.total_profit.partial_cmp(&b.total_profit).unwrap())
        .cloned();

    // Get top 5 per tracking ID
    let mut by_tracking_id_vec: Vec<(String, Vec<ProductStats>)> = Vec::new();
    for (tracking_id, products) in by_tracking_id {
        let mut top: Vec<_> = products.values().cloned().collect();
        top.sort_by(|a, b| b.total_profit.partial_cmp(&a.total_profit).unwrap());
        top.truncate(5);
        by_tracking_id_vec.push((tracking_id, top));
    }

    // Sort by tracking ID for consistent display
    by_tracking_id_vec.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(ReportResults {
        top_overall,
        by_tracking_id: by_tracking_id_vec,
    })
}

struct ReportApp {
    selected_file: Option<String>,
    results: Option<ReportResults>,
    error_message: Option<String>,
}

impl Default for ReportApp {
    fn default() -> Self {
        Self {
            selected_file: None,
            results: None,
            error_message: None,
        }
    }
}

impl eframe::App for ReportApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Affiliate Report Analyzer");
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
                    match process_csv(&path_str) {
                        Ok(results) => {
                            self.results = Some(results);
                            self.error_message = None;
                        }
                        Err(e) => {
                            self.error_message = Some(format!("Error: {}", e));
                            self.results = None;
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
                    // Top overall product
                    ui.heading("Top Performing Product Overall");
                    ui.add_space(5.0);

                    if let Some(p) = &results.top_overall {
                        ui.label(format!("ASIN: {}", p.asin));
                        ui.label(format!("Name: {}", p.name));
                        ui.label(format!("Profit: ${:.2}", p.total_profit));
                    } else {
                        ui.label("No products found");
                    }

                    ui.add_space(20.0);
                    ui.separator();
                    ui.add_space(20.0);

                    // Top 5 per tracking ID
                    ui.heading("Top 5 Products Per Tracking ID");
                    ui.add_space(10.0);

                    for (tracking_id, products) in &results.by_tracking_id {
                        ui.group(|ui| {
                            ui.strong(format!("Tracking ID: {}", tracking_id));
                            ui.add_space(5.0);

                            for (i, p) in products.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.label(format!("{}.", i + 1));
                                    ui.vertical(|ui| {
                                        ui.label(format!("{} - {}", p.asin, p.name));
                                        ui.label(format!("Profit: ${:.2}", p.total_profit));
                                    });
                                });

                                if i < products.len() - 1 {
                                    ui.add_space(5.0);
                                }
                            }
                        });
                        ui.add_space(10.0);
                    }
                });
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([600.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Affiliate Report Analyzer",
        options,
        Box::new(|_cc| Ok(Box::<ReportApp>::default())),
    )
}

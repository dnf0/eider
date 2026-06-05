use crate::{config::EiderConfig, stac, ui, ui::OutputMode};
use color_eyre::eyre::{eyre, Result as EyreResult, WrapErr};

use owo_colors::OwoColorize;

#[derive(Clone)]
struct SelectOption {
    id: String,
    display: String,
}

impl std::fmt::Display for SelectOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display)
    }
}

fn build_stac_query(
    collection: &str,
    bbox: Option<&String>,
    datetime: Option<&String>,
) -> EyreResult<serde_json::Value> {
    let mut payload = serde_json::json!({
        "collections": [collection],
        "limit": 10
    });

    if let Some(b) = bbox {
        let bbox_arr: Vec<f64> = b
            .split(',')
            .map(|s| s.trim().parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
            .wrap_err("Failed to parse bbox coordinates as floats")?;
        if bbox_arr.len() == 4 {
            payload
                .as_object_mut()
                .unwrap()
                .insert("bbox".to_string(), serde_json::json!(bbox_arr));
        } else {
            return Err(eyre!(
                "bbox must be 4 comma-separated numbers (min_lon, min_lat, max_lon, max_lat)"
            ));
        }
    }

    if let Some(dt) = datetime {
        payload
            .as_object_mut()
            .unwrap()
            .insert("datetime".to_string(), serde_json::json!(dt));
    }
    Ok(payload)
}

fn is_supported_asset(asset: &serde_json::Value) -> bool {
    let t = asset.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let href = asset.get("href").and_then(|h| h.as_str()).unwrap_or("");
    let is_zarr = t.contains("zarr") || href.ends_with(".zarr") || href.contains(".zarr/");
    let is_cog = t.contains("tiff")
        || t.contains("cog")
        || href.ends_with(".tif")
        || href.ends_with(".tiff");

    is_zarr || is_cog
}

fn extract_assets(
    assets: &serde_json::Map<String, serde_json::Value>,
    found_options: &mut Vec<SelectOption>,
) {
    for (_, asset) in assets {
        if is_supported_asset(asset) {
            if let Some(href) = asset.get("href").and_then(|h| h.as_str()) {
                let title = asset.get("title").and_then(|t| t.as_str()).unwrap_or(href);
                let mut desc = asset
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .replace('\n', " ");
                if desc.len() > 80 {
                    desc.truncate(77);
                    desc.push_str("...");
                }

                let mut display = if desc.is_empty() {
                    title.bold().cyan().to_string()
                } else {
                    format!("{} - {}", title.bold().cyan(), desc.italic())
                };

                if found_options.len() % 2 == 1 {
                    display = display.on_truecolor(30, 30, 30).to_string();
                }

                found_options.push(SelectOption {
                    id: href.to_string(),
                    display,
                });
            }
        }
    }
}

fn parse_search_results(stac_response: &serde_json::Value) -> Vec<SelectOption> {
    let mut found_options = Vec::new();

    // Check features (items)
    if let Some(features) = stac_response.get("features").and_then(|f| f.as_array()) {
        for feature in features {
            if let Some(assets) = feature.get("assets").and_then(|a| a.as_object()) {
                extract_assets(assets, &mut found_options);
            }
        }
    }

    // Check if the response itself is a collection with assets
    if let Some(assets) = stac_response.get("assets").and_then(|a| a.as_object()) {
        extract_assets(assets, &mut found_options);
    }

    found_options
}

fn output_json_results(options: &[SelectOption]) {
    let uris: Vec<String> = options.iter().map(|o| o.id.clone()).collect();
    let json_out = serde_json::json!({
        "status": "success",
        "uris": uris
    });
    println!("{}", json_out);
}

fn get_selected_api(
    api: Option<String>,
    mode: OutputMode,
    config: &EiderConfig,
) -> EyreResult<String> {
    if let Some(a) = api {
        return Ok(a);
    }
    if !mode.is_human() {
        return Err(eyre!("--api is required in non-interactive mode"));
    }

    let providers = stac::get_stac_providers(config);
    let mut provider_options = Vec::new();
    for (i, p) in providers.iter().enumerate() {
        let mut display = p.to_string();
        if i % 2 == 1 {
            display = display.on_truecolor(30, 30, 30).to_string();
        }
        let id = p.split(" - ").next().unwrap().to_string();
        provider_options.push(SelectOption { id, display });
    }

    let mut select = inquire::Select::new("Select a STAC Provider:", provider_options);
    select.scorer = &|input, _, string_value, _| {
        let input = input.to_lowercase();
        let val = string_value.to_lowercase();
        if input.split_whitespace().all(|word| val.contains(word)) {
            Some(1)
        } else {
            None
        }
    };
    let selection = select.prompt()?;

    Ok(selection.id)
}

async fn get_selected_collection(
    client: &reqwest::Client,
    selected_api: &str,
    current_collection: Option<&String>,
    mode: OutputMode,
) -> EyreResult<Option<String>> {
    if let Some(c) = current_collection {
        return Ok(Some(c.clone()));
    }

    let collections_url = if selected_api.ends_with("/collections") {
        selected_api.to_string()
    } else {
        format!("{}/collections", selected_api.trim_end_matches('/'))
    };

    let res = client
        .get(&collections_url)
        .send()
        .await
        .wrap_err("Failed to fetch collections from STAC API")?;
    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        return Err(eyre!("STAC API returned {}: {}", status, text));
    }

    let collections_response: serde_json::Value = res
        .json()
        .await
        .wrap_err("Failed to parse collections response")?;

    let mut collection_options = Vec::new();
    let mut collection_ids = Vec::new();

    if let Some(collections) = collections_response
        .get("collections")
        .and_then(|c| c.as_array())
    {
        for col in collections {
            if let Some(id) = col.get("id").and_then(|id| id.as_str()) {
                let mut has_supported_data = false;
                if let Some(assets) = col.get("assets").and_then(|a| a.as_object()) {
                    has_supported_data = assets.values().any(is_supported_asset);
                }
                if !has_supported_data {
                    if let Some(item_assets) = col.get("item_assets").and_then(|a| a.as_object()) {
                        has_supported_data = item_assets.values().any(is_supported_asset);
                    }
                }
                if !has_supported_data {
                    continue; // Skip collections that don't declare Zarr or COG assets
                }

                let title = col.get("title").and_then(|t| t.as_str()).unwrap_or(id);
                let mut desc = col
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .replace('\n', " ");
                if desc.len() > 80 {
                    desc.truncate(77);
                    desc.push_str("...");
                }

                let mut display = if desc.is_empty() {
                    title.bold().cyan().to_string()
                } else {
                    format!("{} - {}", title.bold().cyan(), desc.italic())
                };

                if collection_options.len() % 2 == 1 {
                    display = display.on_truecolor(30, 30, 30).to_string();
                }
                collection_options.push(SelectOption {
                    id: id.to_string(),
                    display,
                });
                collection_ids.push(id.to_string());
            }
        }
    }

    if collection_ids.is_empty() {
        return Err(eyre!("No collections found at {}", collections_url));
    }
    if !mode.is_human() {
        if mode == OutputMode::AgentJson {
            let json_out = serde_json::json!({
                "status": "success",
                "collections": collection_ids
            });
            println!("{}", json_out);
            return Ok(None);
        } else {
            return Err(eyre!(
                "--collection is required in non-interactive mode. Available collections: {:?}",
                collection_ids
            ));
        }
    }

    let mut select =
        inquire::Select::new("Select a STAC Collection to search:", collection_options)
            .with_page_size(10);
    select.scorer = &|input, _, string_value, _| {
        let input = input.to_lowercase();
        let val = string_value.to_lowercase();
        if input.split_whitespace().all(|word| val.contains(word)) {
            Some(1)
        } else {
            None
        }
    };
    let selection = select.prompt()?;

    Ok(Some(selection.id))
}

pub async fn run_search(
    api: Option<String>,
    collection: Option<String>,
    bbox: Option<String>,
    datetime: Option<String>,
    mode: OutputMode,
    config: &EiderConfig,
) -> EyreResult<()> {
    let client = reqwest::Client::new();
    let selected_api = get_selected_api(api, mode, config)?;

    let current_collection = collection.clone();
    let selected_collection =
        match get_selected_collection(&client, &selected_api, current_collection.as_ref(), mode)
            .await?
        {
            Some(c) => c,
            None => return Ok(()),
        };

    let payload = build_stac_query(&selected_collection, bbox.as_ref(), datetime.as_ref())?;

    let mut search_api = selected_api.clone();
    if !search_api.ends_with("/search") {
        search_api = format!("{}/search", search_api.trim_end_matches('/'));
    }

    if mode.is_human() {
        eprintln!("Querying STAC API: {}", search_api);
    }

    let res = client
        .post(&search_api)
        .json(&payload)
        .send()
        .await
        .wrap_err("Failed to send request to STAC API")?;
    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        return Err(eyre!("STAC API returned {}: {}", status, text));
    }

    let mut stac_response: serde_json::Value = res
        .json()
        .await
        .wrap_err("Failed to parse STAC API response")?;

    // If the `/search` response returned no features (or it's a dataset where the zarr is attached to the collection),
    // let's fetch the collection itself to see if it has the assets.
    if let Some(features) = stac_response.get("features").and_then(|f| f.as_array()) {
        if features.is_empty() {
            // Fetch the collection specifically
            let collection_url = format!(
                "{}/collections/{}",
                selected_api.trim_end_matches('/'),
                selected_collection
            );
            if let Ok(col_res) = client.get(&collection_url).send().await {
                if let Ok(col_json) = col_res.json::<serde_json::Value>().await {
                    stac_response = col_json;
                }
            }
        }
    }

    let found_options = parse_search_results(&stac_response);

    if !mode.is_human() {
        if mode == OutputMode::AgentJson {
            output_json_results(&found_options);
        } else {
            for opt in &found_options {
                println!("{}", opt.id);
            }
        }
    } else if found_options.is_empty() {
        return Err(eyre!(
            "No Zarr or COG URIs found in collection {}.",
            selected_collection
        ));
    } else {
        let selection_id = if found_options.len() == 1 {
            found_options[0].id.clone()
        } else {
            let prompt_msg = format!(
                "Found {} Data URIs. Select a dataset to use:",
                found_options.len()
            );
            let mut select = inquire::Select::new(&prompt_msg, found_options).with_page_size(10);
            select.scorer = &|input, _, string_value, _| {
                let input = input.to_lowercase();
                let val = string_value.to_lowercase();
                if input.split_whitespace().all(|word| val.contains(word)) {
                    Some(1)
                } else {
                    None
                }
            };
            let chosen = select.prompt()?;
            chosen.id
        };

        // Resolve the specific channel/array from the Zarr group
        let resolved_uri = ui::prompt_zarr_uri(&selection_id, mode).await?;

        // Just output the URL cleanly to stdout to support seamless piping
        println!("{}", resolved_uri);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_query_minimal() {
        let q = build_stac_query("era5", None, None).unwrap();
        assert_eq!(q["collections"][0], "era5");
        assert_eq!(q["limit"], 10);
        assert!(q.get("bbox").is_none());
    }

    #[test]
    fn build_query_with_valid_bbox() {
        let b = "-10,-5,10,5".to_string();
        let q = build_stac_query("c", Some(&b), None).unwrap();
        assert_eq!(q["bbox"], json!([-10.0, -5.0, 10.0, 5.0]));
    }

    #[test]
    fn build_query_rejects_wrong_bbox_len() {
        let b = "1,2,3".to_string();
        assert!(build_stac_query("c", Some(&b), None).is_err());
    }

    #[test]
    fn build_query_rejects_non_numeric_bbox() {
        let b = "a,b,c,d".to_string();
        assert!(build_stac_query("c", Some(&b), None).is_err());
    }

    #[test]
    fn build_query_with_datetime() {
        let dt = "2020-01-01/2020-12-31".to_string();
        let q = build_stac_query("c", None, Some(&dt)).unwrap();
        assert_eq!(q["datetime"], "2020-01-01/2020-12-31");
    }

    #[test]
    fn supported_asset_detects_zarr_and_cog() {
        assert!(is_supported_asset(
            &json!({"type": "application/vnd+zarr", "href": ""})
        ));
        assert!(is_supported_asset(
            &json!({"type": "", "href": "x/data.zarr/"})
        ));
        assert!(is_supported_asset(
            &json!({"type": "image/tiff", "href": ""})
        ));
        assert!(is_supported_asset(&json!({"type": "", "href": "a.tif"})));
    }

    #[test]
    fn supported_asset_rejects_other() {
        assert!(!is_supported_asset(
            &json!({"type": "application/json", "href": "a.json"})
        ));
    }

    #[test]
    fn parse_results_from_features() {
        let resp = json!({
            "features": [{
                "assets": { "data": { "type": "application/vnd+zarr", "href": "s3://b/x.zarr" } }
            }]
        });
        let opts = parse_search_results(&resp);
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].id, "s3://b/x.zarr");
    }

    #[test]
    fn parse_results_from_collection_assets() {
        let resp = json!({
            "assets": { "data": { "type": "application/vnd+zarr", "href": "s3://b/y.zarr" } }
        });
        let opts = parse_search_results(&resp);
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].id, "s3://b/y.zarr");
    }

    #[test]
    fn parse_results_skips_unsupported() {
        let resp = json!({
            "assets": { "thumb": { "type": "image/png", "href": "a.png" } }
        });
        assert!(parse_search_results(&resp).is_empty());
    }
}

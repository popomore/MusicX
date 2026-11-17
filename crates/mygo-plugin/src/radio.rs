//! Select only explicit personal FM or daily recommendation routes. A provider's
//! home page can also contain charts, new releases and history, so never import it wholesale.
use crate::protocol::{
    MAX_QUEUE_TRACKS, SourceCapability as Capability, SourcePageRequest, Track, tr,
};
use crate::{capabilities, discovery, music_sources};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

fn personal_station(id: &str) -> bool {
    matches!(
        id,
        "DEFAULT" | "normal" | "guess" | "fm" | "personal" | "personal_fm" | "private_fm"
    )
}

fn personal_page(id: &str) -> bool {
    matches!(
        id,
        "daily"
            | "radar"
            | "daily-recommendations"
            | "daily_songs"
            | "dailySongs"
            | "recommend_songs"
    )
}

fn decode(rows: &[Value]) -> Result<Vec<Track>> {
    ensure!(
        rows.len() <= MAX_QUEUE_TRACKS,
        tr!("Source exceeds 10,000 tracks")
    );
    rows.iter()
        .filter(|row| row["unavailable"] != true)
        .map(|row| Ok(serde_json::from_value(row.clone())?))
        .collect()
}

fn section(
    value: &Value,
    pages: &mut Vec<String>,
    tracks: &mut Vec<Track>,
    depth: usize,
) -> Result<()> {
    ensure!(depth < 32, tr!("Source response is nested too deeply"));
    if let Some(page) = value["morePage"].as_str().filter(|id| personal_page(id)) {
        if !pages.iter().any(|id| id == page) {
            pages.push(page.into());
        }
        if let Some(rows) = value["tracks"].as_array() {
            tracks.extend(decode(rows)?);
        }
    }
    if let Some(links) = value["links"].as_array() {
        for link in links {
            if let Some(page) = link["page"].as_str().filter(|id| personal_page(id))
                && !pages.iter().any(|id| id == page)
            {
                pages.push(page.into());
            }
        }
    }
    if let Some(sections) = value["sections"].as_array() {
        for child in sections {
            section(child, pages, tracks, depth + 1)?;
        }
    }
    ensure!(
        tracks.len() <= MAX_QUEUE_TRACKS,
        tr!("Source exceeds 10,000 tracks")
    );
    Ok(())
}

pub fn tracks(source: &str) -> Result<Vec<Track>> {
    let info = music_sources::get_source(source)?;
    let mut failure = None;
    if info.supports(Capability::Stations) && info.supports(Capability::StationContinuation) {
        let station = discovery::page(source, SourcePageRequest::Stations {}).and_then(|page| {
            let Some(station) = page
                .collections
                .into_iter()
                .find(|station| personal_station(&station.id))
            else {
                return Ok(Vec::new());
            };
            let value = capabilities::invoke(source, "stationNext", json!({"id":station.id}))?;
            let rows = value
                .as_array()
                .or_else(|| value["tracks"].as_array())
                .context(tr!("The source returned no track list"))?;
            decode(rows)
        });
        match station {
            Ok(tracks) if !tracks.is_empty() => return Ok(tracks),
            Err(error) => failure = Some(error),
            _ => {}
        }
    }
    if info.supports(Capability::Recommendations) {
        let home = capabilities::invoke(source, "home", json!({}));
        match home {
            Ok(home) => {
                let mut pages = Vec::new();
                let mut tracks = Vec::new();
                section(&home, &mut pages, &mut tracks, 0)?;
                if info.supports(Capability::Browse) {
                    for page in pages {
                        let result = capabilities::invoke(source, "browse", json!({"page":page}))
                            .and_then(|value| {
                                // This response belongs to the explicit personal page.
                                fn collect(
                                    value: &Value,
                                    tracks: &mut Vec<Track>,
                                    depth: usize,
                                ) -> Result<()> {
                                    ensure!(
                                        depth < 32,
                                        tr!("Source response is nested too deeply")
                                    );
                                    if let Some(rows) = value["tracks"].as_array() {
                                        tracks.extend(decode(rows)?);
                                    }
                                    if let Some(sections) = value["sections"].as_array() {
                                        for section in sections {
                                            collect(section, tracks, depth + 1)?;
                                        }
                                    }
                                    ensure!(
                                        tracks.len() <= MAX_QUEUE_TRACKS,
                                        tr!("Source exceeds 10,000 tracks")
                                    );
                                    Ok(())
                                }
                                let mut tracks = Vec::new();
                                collect(&value, &mut tracks, 0)?;
                                Ok(tracks)
                            });
                        match result {
                            Ok(tracks) if !tracks.is_empty() => return Ok(tracks),
                            Err(error) => failure = Some(error),
                            _ => {}
                        }
                    }
                }
                if !tracks.is_empty() {
                    return Ok(tracks);
                }
            }
            Err(error) => failure = Some(error),
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(Vec::new()),
    }
}

//! Weather from Open-Meteo (no API key, no account): the user picks a city by name (geocoding);
//! the network address is never used to locate anyone. Responses are cached per place and
//! refreshed every 30 minutes; offline, the cache shows with its age.
use serde_json::Value;
use shared::widgets::{Place, Units};

pub const REFRESH_SECS: u64 = 30 * 60;
pub const FORECAST_HOURS: usize = 6;
pub const FORECAST_DAYS: usize = 6;
/// Column widths (characters) of the daily rows: "Today", "-12°".
pub const DAY_CHARS: i32 = 5;
pub const TEMP_CHARS: i32 = 4;

/// `application/x-www-form-urlencoded` component encoding.
pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn geocode_url(query: &str) -> String {
    format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=6&language=en&format=json",
        encode(query.trim())
    )
}

pub fn forecast_url(p: &Place, units: Units) -> String {
    let mut u = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}\
         &current=temperature_2m,weather_code,is_day\
         &hourly=temperature_2m,weather_code,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min\
         &timezone=auto&forecast_days={FORECAST_DAYS}",
        p.lat, p.lon
    );
    if units == Units::Fahrenheit {
        u.push_str("&temperature_unit=fahrenheit");
    }
    u
}

/// Geocoding results: "Lisbon" with "Lisbon District, Portugal".
pub fn parse_places(json: &str) -> Vec<Place> {
    let v: Value = serde_json::from_str(json).unwrap_or_default();
    v["results"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let region: Vec<&str> = ["admin1", "country"]
                        .iter()
                        .filter_map(|k| r[*k].as_str())
                        .filter(|s| !s.is_empty())
                        .collect();
                    Some(Place {
                        name: r["name"].as_str()?.to_string(),
                        region: region.join(", "),
                        lat: r["latitude"].as_f64()?,
                        lon: r["longitude"].as_f64()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Fahrenheit where people use it (territory of `LANG`/`LC_MEASUREMENT`), else Celsius.
pub fn units_for_locale(locale: &str) -> Units {
    let territory = locale
        .split(['.', '@'])
        .next()
        .and_then(|l| l.split('_').nth(1))
        .unwrap_or("");
    if ["US", "LR", "MM", "BS", "BZ", "KY", "PW", "FM", "MH"].contains(&territory) {
        Units::Fahrenheit
    } else {
        Units::Celsius
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hour {
    /// "Now", "15:00".
    pub label: String,
    pub temp: f64,
    pub code: u8,
    pub day: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    /// "Today", "Wed".
    pub label: String,
    pub low: f64,
    pub high: f64,
    pub code: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Forecast {
    pub temp: f64,
    pub code: u8,
    pub day: bool,
    pub high: f64,
    pub low: f64,
    pub hours: Vec<Hour>,
    pub days: Vec<Day>,
}

/// Day of week (0 = Monday) of a proleptic Gregorian date.
pub fn weekday(y: i32, m: u32, d: u32) -> u32 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let sunday0 = (y + y / 4 - y / 100 + y / 400 + T[(m - 1) as usize] + d as i32).rem_euclid(7);
    ((sunday0 + 6) % 7) as u32
}

const DAY_ABBR: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn ymd(s: &str) -> Option<(i32, u32, u32)> {
    let mut p = s.get(..10)?.split('-');
    Some((
        p.next()?.parse().ok()?,
        p.next()?.parse().ok()?,
        p.next()?.parse().ok()?,
    ))
}

/// Open-Meteo forecast JSON (`timezone=auto`: times are the place's local time).
pub fn parse_forecast(json: &str) -> Option<Forecast> {
    let v: Value = serde_json::from_str(json).ok()?;
    let c = &v["current"];
    let nums = |a: &Value| -> Vec<f64> {
        a.as_array()
            .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect())
            .unwrap_or_default()
    };
    let strs = |a: &Value| -> Vec<String> {
        a.as_array()
            .map(|a| {
                a.iter()
                    .map(|x| x.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    let now_time = c["time"].as_str()?;
    let now_hour = now_time.get(..13)?; // "2026-10-27T04"
    let h = &v["hourly"];
    let (ht, htemp, hcode, hday) = (
        strs(&h["time"]),
        nums(&h["temperature_2m"]),
        nums(&h["weather_code"]),
        nums(&h["is_day"]),
    );
    let start = ht.iter().position(|t| t.starts_with(now_hour))?;
    let mut hours = Vec::new();
    for (i, time) in ht.iter().enumerate().skip(start).take(FORECAST_HOURS) {
        let (Some(t), Some(c), Some(d)) = (htemp.get(i), hcode.get(i), hday.get(i)) else {
            break;
        };
        hours.push(Hour {
            label: if i == start {
                "Now".into()
            } else {
                format!("{}:00", time.get(11..13).unwrap_or("--"))
            },
            temp: *t,
            code: *c as u8,
            day: *d > 0.5,
        });
    }
    let d = &v["daily"];
    let (dt, dcode, dmax, dmin) = (
        strs(&d["time"]),
        nums(&d["weather_code"]),
        nums(&d["temperature_2m_max"]),
        nums(&d["temperature_2m_min"]),
    );
    let today = now_time.get(..10)?;
    let first = dt.iter().position(|t| t == today).unwrap_or(0);
    let days: Vec<Day> = (first..dt.len().min(first + FORECAST_DAYS))
        .filter_map(|i| {
            let (y, m, dd) = ymd(&dt[i])?;
            Some(Day {
                label: if i == first {
                    "Today".into()
                } else {
                    DAY_ABBR[weekday(y, m, dd) as usize].into()
                },
                low: *dmin.get(i)?,
                high: *dmax.get(i)?,
                code: *dcode.get(i)? as u8,
            })
        })
        .collect();
    let today = days.first()?;
    Some(Forecast {
        temp: c["temperature_2m"].as_f64()?,
        code: c["weather_code"].as_f64()? as u8,
        day: c["is_day"].as_f64().unwrap_or(1.0) > 0.5,
        high: today.high,
        low: today.low,
        hours,
        days,
    })
}

/// WMO weather code -> (label, symbolic icon).
pub fn condition(code: u8, day: bool) -> (&'static str, &'static str) {
    let (clear, few) = if day {
        ("weather-clear-symbolic", "weather-few-clouds-symbolic")
    } else {
        (
            "weather-clear-night-symbolic",
            "weather-few-clouds-night-symbolic",
        )
    };
    match code {
        0 => (if day { "Sunny" } else { "Clear" }, clear),
        1 => ("Mostly Clear", few),
        2 => ("Partly Cloudy", few),
        3 => ("Cloudy", "weather-overcast-symbolic"),
        45 | 48 => ("Fog", "weather-fog-symbolic"),
        51..=57 => ("Drizzle", "weather-showers-scattered-symbolic"),
        61 | 63 | 66 => ("Rain", "weather-showers-symbolic"),
        65 | 67 => ("Heavy Rain", "weather-showers-symbolic"),
        71..=77 | 85 | 86 => ("Snow", "weather-snow-symbolic"),
        80..=82 => ("Showers", "weather-showers-scattered-symbolic"),
        95..=99 => ("Thunderstorms", "weather-storm-symbolic"),
        _ => ("Cloudy", "weather-overcast-symbolic"),
    }
}

/// "18°" (rounded, no unit letter: macOS style).
pub fn degrees(t: f64) -> String {
    let r = t.round();
    format!("{}°", if r == 0.0 { 0.0 } else { r })
}

/// Cache entry: the raw response and when it was fetched (unix s).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cached {
    pub fetched: u64,
    pub units: Units,
    pub body: String,
}

/// `<cache home>/wolos/weather/<lat>,<lon>.json`.
pub fn cache_path(cache_home: &std::path::Path, p: &Place) -> std::path::PathBuf {
    cache_home
        .join("wolos/weather")
        .join(format!("{:.3},{:.3}.json", p.lat, p.lon))
}

pub fn is_stale(c: &Cached, units: Units, now: u64) -> bool {
    c.units != units || now.saturating_sub(c.fetched) >= REFRESH_SECS
}

/// Shown when the last fetch failed: "Updated 40 min ago".
pub fn age_label(fetched: u64, now: u64) -> String {
    let m = now.saturating_sub(fetched) / 60;
    match m {
        0 => "Updated just now".into(),
        1..=59 => format!("Updated {m} min ago"),
        60..=1439 => format!("Updated {} h ago", m / 60),
        _ => format!("Updated {} d ago", m / 1440),
    }
}

/// What a refresh found.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Fresh(Forecast),
    /// Fetch failed; the cached forecast with its age.
    Stale(Forecast, String),
    /// Fetch failed and nothing usable is cached.
    Offline,
}

/// Cache first (within 30 minutes and the same units), else `fetch`, then the cache again as a
/// fallback. A good response is written back. Blocking: call off the UI thread.
pub fn load(
    cache_home: &std::path::Path,
    p: &Place,
    units: Units,
    now: u64,
    force: bool,
    fetch: impl Fn(&str) -> Result<Vec<u8>, String>,
) -> Outcome {
    let path = cache_path(cache_home, p);
    let cached: Option<Cached> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let parsed = cached
        .as_ref()
        .filter(|c| c.units == units)
        .and_then(|c| parse_forecast(&c.body).map(|f| (c.fetched, f)));
    if let (false, Some(c), Some((_, f))) = (force, &cached, &parsed)
        && !is_stale(c, units, now)
    {
        return Outcome::Fresh(f.clone());
    }
    let got = fetch(&forecast_url(p, units))
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|body| parse_forecast(&body).map(|f| (body, f)));
    match (got, parsed) {
        (Some((body, f)), _) => {
            let c = Cached {
                fetched: now,
                units,
                body,
            };
            if let (Some(dir), Ok(json)) = (path.parent(), serde_json::to_string(&c)) {
                let tmp = path.with_extension("json.tmp");
                let _ = std::fs::create_dir_all(dir)
                    .and_then(|_| std::fs::write(&tmp, json))
                    .and_then(|_| std::fs::rename(&tmp, &path));
            }
            Outcome::Fresh(f)
        }
        (None, Some((fetched, f))) => Outcome::Stale(f, age_label(fetched, now)),
        (None, None) => Outcome::Offline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed real response (Lisbon, 2026-10-27 04:15 local).
    const FC: &str = r#"{"timezone":"Europe/Lisbon",
      "current":{"time":"2026-10-27T04:15","interval":900,"temperature_2m":11.6,"weather_code":2,"is_day":0},
      "hourly":{"time":["2026-10-27T03:00","2026-10-27T04:00","2026-10-27T05:00","2026-10-27T06:00","2026-10-27T07:00","2026-10-27T08:00","2026-10-27T09:00","2026-10-27T10:00"],
                "temperature_2m":[11.9,11.6,11.2,11.0,10.9,11.8,13.4,15.1],
                "weather_code":[1,2,2,3,3,61,3,0],
                "is_day":[0,0,0,0,0,1,1,1]},
      "daily":{"time":["2026-10-27","2026-10-28","2026-10-29"],
               "weather_code":[61,3,0],"temperature_2m_max":[19.4,18.0,21.2],"temperature_2m_min":[10.9,12.1,13.0]}}"#;

    #[test]
    fn parses_now_next_hours_and_days() {
        let f = parse_forecast(FC).unwrap();
        assert_eq!((f.temp, f.code, f.day), (11.6, 2, false));
        assert_eq!((f.low, f.high), (10.9, 19.4));
        let hours: Vec<(&str, f64)> = f.hours.iter().map(|h| (h.label.as_str(), h.temp)).collect();
        assert_eq!(
            hours,
            [
                ("Now", 11.6),
                ("05:00", 11.2),
                ("06:00", 11.0),
                ("07:00", 10.9),
                ("08:00", 11.8),
                ("09:00", 13.4)
            ]
        );
        assert!(f.hours[4].day && !f.hours[0].day);
        let days: Vec<&str> = f.days.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(days, ["Today", "Wed", "Thu"]);
        assert_eq!(f.days[2].code, 0);
    }

    #[test]
    fn rejects_responses_without_current_conditions() {
        for bad in [
            "",
            "{}",
            r#"{"current": {"time": "2026-10-27T04:15"}}"#,
            r#"{"error": true, "reason": "x"}"#,
        ] {
            assert_eq!(parse_forecast(bad), None, "{bad}");
        }
    }

    #[test]
    fn places_come_from_the_name_the_user_typed() {
        let json = r#"{"results":[{"name":"Lisbon","latitude":38.72509,"longitude":-9.1498,"country":"Portugal","admin1":"Lisbon District"},
                                 {"name":"Lisbon","latitude":40.772,"longitude":-80.76813,"country":"United States","admin1":"Ohio"},
                                 {"name":"Broken"}]}"#;
        let p = parse_places(json);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].region, "Lisbon District, Portugal");
        assert_eq!(p[1].lat, 40.772);
        assert!(parse_places("{}").is_empty() && parse_places("nope").is_empty());
        assert_eq!(
            geocode_url(" São Paulo "),
            "https://geocoding-api.open-meteo.com/v1/search?name=S%C3%A3o+Paulo&count=6&language=en&format=json"
        );
        let u = forecast_url(&p[0], Units::Fahrenheit);
        assert!(u.starts_with(
            "https://api.open-meteo.com/v1/forecast?latitude=38.7251&longitude=-9.1498&"
        ));
        assert!(u.ends_with("&temperature_unit=fahrenheit"));
        assert!(!forecast_url(&p[0], Units::Celsius).contains("fahrenheit"));
    }

    #[test]
    fn units_follow_the_locale() {
        for (l, u) in [
            ("en_US.UTF-8", Units::Fahrenheit),
            ("en_GB.UTF-8", Units::Celsius),
            ("pt_BR.UTF-8", Units::Celsius),
            ("C", Units::Celsius),
            ("", Units::Celsius),
        ] {
            assert_eq!(units_for_locale(l), u, "{l}");
        }
    }

    #[test]
    fn conditions_and_degrees() {
        assert_eq!(condition(0, true), ("Sunny", "weather-clear-symbolic"));
        assert_eq!(
            condition(0, false),
            ("Clear", "weather-clear-night-symbolic")
        );
        assert_eq!(condition(95, true).1, "weather-storm-symbolic");
        assert_eq!(condition(200, true).0, "Cloudy");
        assert_eq!(degrees(17.5), "18°");
        assert_eq!(degrees(-0.4), "0°");
        assert_eq!(degrees(-3.6), "-4°");
        assert_eq!(weekday(2026, 10, 26), 0, "Monday");
        assert_eq!(weekday(2000, 2, 29), 1);
    }

    #[test]
    fn cache_then_network_then_stale_cache() {
        let dir = std::env::temp_dir().join(format!("wol-weather-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = Place {
            name: "Lisbon".into(),
            region: String::new(),
            lat: 38.725,
            lon: -9.15,
        };
        let calls = std::cell::Cell::new(0);
        let ok = |_: &str| {
            calls.set(calls.get() + 1);
            Ok(FC.as_bytes().to_vec())
        };
        let down = |_: &str| -> Result<Vec<u8>, String> { Err("offline".into()) };
        let t0 = 1_000_000;
        assert_eq!(
            load(&dir, &p, Units::Celsius, t0, false, down),
            Outcome::Offline
        );
        assert!(matches!(
            load(&dir, &p, Units::Celsius, t0, false, ok),
            Outcome::Fresh(_)
        ));
        assert_eq!(calls.get(), 1);
        assert!(matches!(
            load(&dir, &p, Units::Celsius, t0 + 600, false, ok),
            Outcome::Fresh(_)
        ));
        assert_eq!(calls.get(), 1, "fresh cache: no request");
        assert!(matches!(
            load(&dir, &p, Units::Celsius, t0 + 600, true, ok),
            Outcome::Fresh(_)
        ));
        assert_eq!(calls.get(), 2, "forced refresh asks again");
        match load(&dir, &p, Units::Celsius, t0 + 3000, false, down) {
            Outcome::Stale(_, age) => assert_eq!(age, "Updated 40 min ago"),
            o => panic!("{o:?}"),
        }
        assert_eq!(
            load(&dir, &p, Units::Fahrenheit, t0 + 60, false, down),
            Outcome::Offline,
            "a cache in other units is not shown"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cache_ages() {
        let c = Cached {
            fetched: 1000,
            units: Units::Celsius,
            body: String::new(),
        };
        assert!(!is_stale(&c, Units::Celsius, 1000 + REFRESH_SECS - 1));
        assert!(is_stale(&c, Units::Celsius, 1000 + REFRESH_SECS));
        assert!(is_stale(&c, Units::Fahrenheit, 1000));
        for (age, want) in [
            (10, "Updated just now"),
            (40 * 60, "Updated 40 min ago"),
            (3 * 3600, "Updated 3 h ago"),
            (2 * 86400, "Updated 2 d ago"),
        ] {
            assert_eq!(age_label(1000, 1000 + age), want);
        }
        let p = Place {
            name: "x".into(),
            region: String::new(),
            lat: 38.72509,
            lon: -9.1498,
        };
        assert_eq!(
            cache_path(std::path::Path::new("/c"), &p),
            std::path::Path::new("/c/wolos/weather/38.725,-9.150.json")
        );
    }
}

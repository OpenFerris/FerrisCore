use super::*;

pub(super) fn validate_link(
    field_name: &str,
    value: &Value,
    field_meta: &ferris_schema::registry::FieldMeta,
) -> Result<()> {
    // A link's *existence* is validated by the engine's Link checks; here we
    // only reject an empty value on a field that names a target DocType, which
    // no target table could ever contain.
    if let Some(link_value) = value.as_str() {
        if link_value.is_empty() && field_meta.options.is_some() {
            return Err(ferris_core::Error::Validation(
                ferris_core::ValidationError::Field {
                    field: field_name.to_string(),
                    message: "Link value cannot be empty".to_string(),
                },
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_select(
    field_name: &str,
    value: &Value,
    field_meta: &ferris_schema::registry::FieldMeta,
) -> Result<()> {
    if let Some(select_value) = value.as_str() {
        if let Some(options) = &field_meta.options {
            let valid_options: Vec<String> = options
                .split('\n')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            if !valid_options.is_empty() && !valid_options.iter().any(|o| o == select_value) {
                return Err(ferris_core::Error::Validation(
                    ferris_core::ValidationError::Field {
                        field: field_name.to_string(),
                        message: format!(
                            "Invalid value '{}'. Valid options: {}",
                            select_value,
                            valid_options.join(", ")
                        ),
                    },
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_email(field_name: &str, value: &Value) -> Result<()> {
    if let Some(email) = value.as_str() {
        if !is_valid_email(email) {
            return Err(ferris_core::Error::Validation(
                ferris_core::ValidationError::Field {
                    field: field_name.to_string(),
                    message: "Invalid email format".to_string(),
                },
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_phone(field_name: &str, value: &Value) -> Result<()> {
    if let Some(phone) = value.as_str() {
        if !is_valid_phone(phone) {
            return Err(ferris_core::Error::Validation(
                ferris_core::ValidationError::Field {
                    field: field_name.to_string(),
                    message: "Invalid phone format".to_string(),
                },
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_url(field_name: &str, value: &Value) -> Result<()> {
    if let Some(url) = value.as_str() {
        if !is_valid_url(url) {
            return Err(ferris_core::Error::Validation(
                ferris_core::ValidationError::Field {
                    field: field_name.to_string(),
                    message: "Invalid URL format".to_string(),
                },
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_number(
    field_name: &str,
    value: &Value,
    field_meta: &ferris_schema::registry::FieldMeta,
) -> Result<()> {
    // A7: this used to be a no-op once a value parsed as f64 — `min_value`,
    // `max_value` and `precision` were never checked, so a `Float` with
    // `min_value: "0"` accepted -999999 and a Currency with `precision: 2`
    // stored 1.239.
    let num = number_value(field_name, value)?;

    if let Some(min_val) = &field_meta.min_value
        && let Ok(min) = min_val.parse::<f64>()
        && num < min
    {
        return Err(field_error(field_name, format!("Value must be >= {min}")));
    }
    if let Some(max_val) = &field_meta.max_value
        && let Ok(max) = max_val.parse::<f64>()
        && num > max
    {
        return Err(field_error(field_name, format!("Value must be <= {max}")));
    }

    // `precision` used to be read nowhere (`grep precision` found no consumer),
    // so any value passed and 1.239 was stored on a `precision: 2` field.
    // Frappe would round it; Ferris refuses input that would silently lose
    // precision instead.
    if let Some(precision) = field_meta.precision {
        let scale = 10f64.powi(i32::from(precision));
        let rounded = (num * scale).round() / scale;
        if !approximately_equal(num, rounded) {
            return Err(field_error(
                field_name,
                format!("Value cannot have more than {precision} decimal places"),
            ));
        }
    }
    Ok(())
}

/// The number behind a numeric-ish value, parsed once per field.
///
/// Shared by every numeric rule (`Float`, `Percent`, `Rating`, …) so the
/// "numeric strings are honoured, anything else is refused" policy lives in
/// exactly one place — two copies would eventually disagree about what a
/// number is.
fn number_value(field_name: &str, value: &Value) -> Result<f64> {
    let num = match value.as_f64() {
        Some(n) => n,
        None => {
            // Numeric strings from clients are still honoured; anything that is
            // not a number (objects, booleans, garbage strings) is refused.
            match value.as_str().and_then(|s| s.trim().parse::<f64>().ok()) {
                Some(n) => n,
                None => {
                    return Err(field_error(
                        field_name,
                        format!("Value for '{field_name}' must be a number"),
                    ));
                }
            }
        }
    };
    if !num.is_finite() {
        return Err(field_error(
            field_name,
            format!("Value for '{field_name}' must be a finite number"),
        ));
    }
    Ok(num)
}

/// The integer behind an integer-ish value, parsed once per field — the
/// integer twin of [`number_value`], sharing its "strings are honoured"
/// policy.
fn integer_value(field_name: &str, value: &Value) -> Result<i64> {
    match value.as_i64() {
        Some(n) => Ok(n),
        None => match value.as_str().and_then(|s| s.trim().parse::<i64>().ok()) {
            Some(n) => Ok(n),
            None => Err(field_error(
                field_name,
                format!("Value for '{field_name}' must be an integer"),
            )),
        },
    }
}

/// Enforce an inclusive numeric range. `None` on either side means
/// "unbounded" — the caller decides whether the bound is declared metadata or
/// the fieldtype's own default.
fn check_range(field_name: &str, num: f64, min: Option<f64>, max: Option<f64>) -> Result<()> {
    if let Some(min) = min
        && num < min
    {
        return Err(field_error(field_name, format!("Value must be >= {min}")));
    }
    if let Some(max) = max
        && num > max
    {
        return Err(field_error(field_name, format!("Value must be <= {max}")));
    }
    Ok(())
}

/// The field's declared `min_value`/`max_value` as numbers. An absent or
/// unparsable declaration yields `None` and never invents a bound.
fn declared_bounds(field_meta: &ferris_schema::registry::FieldMeta) -> (Option<f64>, Option<f64>) {
    let parse = |raw: &Option<String>| {
        raw.as_deref()
            .and_then(|raw| raw.trim().parse::<f64>().ok())
    };
    (parse(&field_meta.min_value), parse(&field_meta.max_value))
}

pub(super) fn validate_integer(
    field_name: &str,
    value: &Value,
    field_meta: &ferris_schema::registry::FieldMeta,
) -> Result<()> {
    // DOC3: this used to do range checks *inside* `if let Some(num) =
    // value.as_i64()`, so anything `as_i64` could not read — `"abc"`, `true`,
    // `1.5`, `[]`, `{}` — fell straight through to `Ok(())` and an `Int` field
    // accepted every one of them. `validate_field` already skips a non-required
    // `null` (mod.rs:25-27), so from here the value *must* be an integer: parse
    // it the way `validate_number` parses numerics (a numeric string is
    // honoured) and refuse the shape otherwise.
    let num = integer_value(field_name, value)?;

    if let Some(min_val) = &field_meta.min_value
        && let Ok(min) = min_val.parse::<i64>()
        && num < min
    {
        return Err(ferris_core::Error::Validation(
            ferris_core::ValidationError::Field {
                field: field_name.to_string(),
                message: format!("Value must be >= {min}"),
            },
        ));
    }
    if let Some(max_val) = &field_meta.max_value
        && let Ok(max) = max_val.parse::<i64>()
        && num > max
    {
        return Err(ferris_core::Error::Validation(
            ferris_core::ValidationError::Field {
                field: field_name.to_string(),
                message: format!("Value must be <= {max}"),
            },
        ));
    }
    Ok(())
}

pub(super) fn validate_datetime(field_name: &str, value: &Value, fieldtype: &str) -> Result<()> {
    // A7: used to be a no-op, so a `Date` field accepted `{"a":1}`, `12345`
    // and `"no-es-una-fecha"`. Formats follow what Ferris itself writes and
    // reads: RFC 3339 for Datetime, ISO date for Date, `HH:MM[:SS]` for Time.
    let Some(text) = value.as_str() else {
        return Err(field_error(
            field_name,
            format!("{fieldtype} value must be a string"),
        ));
    };
    let valid = match fieldtype {
        "Date" => chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok(),
        "Datetime" => {
            chrono::DateTime::parse_from_rfc3339(text).is_ok()
                || chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").is_ok()
        }
        "Time" => {
            chrono::NaiveTime::parse_from_str(text, "%H:%M:%S").is_ok()
                || chrono::NaiveTime::parse_from_str(text, "%H:%M").is_ok()
        }
        _ => true,
    };
    if !valid {
        return Err(field_error(
            field_name,
            format!("Invalid {fieldtype} format"),
        ));
    }
    Ok(())
}

pub(super) fn validate_boolean(field_name: &str, value: &Value) -> Result<()> {
    if !value.is_null() && value.as_bool().is_none() {
        return Err(ferris_core::Error::Validation(
            ferris_core::ValidationError::Field {
                field: field_name.to_string(),
                message: "Value must be a boolean".to_string(),
            },
        ));
    }
    Ok(())
}

pub(super) fn validate_json(field_name: &str, value: &Value) -> Result<()> {
    if let Value::Object(_) = value {
        Ok(())
    } else if let Value::Array(_) = value {
        Ok(())
    } else if value.is_null() {
        Ok(())
    } else {
        Err(ferris_core::Error::Validation(
            ferris_core::ValidationError::Field {
                field: field_name.to_string(),
                message: "Value must be a JSON object or array".to_string(),
            },
        ))
    }
}

pub(super) fn validate_max_value(field_name: &str, value: &Value, max_val: &str) -> Result<()> {
    // A7: used to be `Ok(())` — `validate_field` calls this for *every* field
    // type that declares `max_value`, so Enforcement came from nowhere. Numeric
    // values are compared; non-numeric values (dates compared as strings, etc.)
    // cannot overflow a numeric max and pass.
    let Some(num) = value.as_f64() else {
        return Ok(());
    };
    if let Ok(max) = max_val.parse::<f64>()
        && num > max
    {
        return Err(field_error(field_name, format!("Value must be <= {max}")));
    }
    Ok(())
}

pub(super) fn validate_min_value(field_name: &str, value: &Value, min_val: &str) -> Result<()> {
    let Some(num) = value.as_f64() else {
        return Ok(());
    };
    if let Ok(min) = min_val.parse::<f64>()
        && num < min
    {
        return Err(field_error(field_name, format!("Value must be >= {min}")));
    }
    Ok(())
}

fn approximately_equal(a: f64, b: f64) -> bool {
    (a - b).abs() <= f64::EPSILON * a.abs().max(b.abs()).max(1.0)
}

// --------------------------------------------------------------------------------------
// the typed fieldtypes that used to fall through the catch-all arm and
// accepted anything at all. Each rule documents the policy it applies; the shared
// parsing helpers above are what keeps those policies from forking.
// --------------------------------------------------------------------------------------

/// `Percent` is a number: the metadata's `min_value`/`max_value`/`precision`
/// are the contract, exactly as for `Float`.
///
/// There is deliberately **no** implicit 0..=100 bound, unlike `Rating`: a
/// -5% refund or a 150% bonus are legal business values that Frappe stores
/// without complaint, and silently rejecting them here would make Ferris the
/// only layer that loses data.
pub(super) fn validate_percent(
    field_name: &str,
    value: &Value,
    field_meta: &ferris_schema::registry::FieldMeta,
) -> Result<()> {
    validate_number(field_name, value, field_meta)
}

/// `Rating` is a 0..=5 star score (the Desk paints half-steps within that
/// range). Unlike `Percent`, a rating outside the star range carries no
/// meaning, so the type ships a default bound — and, unlike the metadata
/// bound, the default is *replaced*, never narrowed, by what the field
/// declares: a field that states its own `min_value`/`max_value` speaks for
/// itself.
pub(super) fn validate_rating(
    field_name: &str,
    value: &Value,
    field_meta: &ferris_schema::registry::FieldMeta,
) -> Result<()> {
    let num = number_value(field_name, value)?;
    let (min, max) = declared_bounds(field_meta);
    check_range(field_name, num, min.or(Some(0.0)), max.or(Some(5.0)))
}

/// `Duration` stores whole seconds — the Desk composes `HH:MM:SS` into a
/// count before writing — so it is an integer field whose type default is 0:
/// an elapsed time cannot be negative. Declared bounds replace the default,
/// as with `Rating`.
pub(super) fn validate_duration(
    field_name: &str,
    value: &Value,
    field_meta: &ferris_schema::registry::FieldMeta,
) -> Result<()> {
    // `as f64` is the only conversion Rust offers from `i64`, and it is
    // exact here: a duration beyond 2^53 seconds (≈285 million years) would
    // already be nonsense, and a bound check keeps its meaning.
    #[allow(clippy::cast_precision_loss)]
    let num = integer_value(field_name, value)? as f64;
    let (min, max) = declared_bounds(field_meta);
    check_range(field_name, num, min.or(Some(0.0)), max)
}

/// `Geolocation` carries `GeoJSON` — the Desk writes a `FeatureCollection`
/// once anything has been drawn — with the historical `"lat,lon"` string
/// still accepted so pre-`GeoJSON` documents keep validating. Clients may post
/// the parsed object directly; all three shapes are honoured, everything
/// else is refused. The shape check is structural (a known `type`, the
/// member array that type requires) rather than a full geometry schema:
/// deeper coordinate validation belongs to whoever draws, not to the store.
pub(super) fn validate_geolocation(field_name: &str, value: &Value) -> Result<()> {
    match value {
        Value::Object(_) => validate_geojson_shape(field_name, value),
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                // Cleared field; `reqd` was already checked upstream.
                return Ok(());
            }
            if let Some((lat, lon)) = text.split_once(',')
                && latlon_in_range(lat, lon)
            {
                return Ok(());
            }
            match serde_json::from_str::<Value>(text) {
                Ok(parsed) => validate_geojson_shape(field_name, &parsed),
                Err(_) => Err(field_error(
                    field_name,
                    "Geolocation must be GeoJSON or a \"lat,lon\" pair",
                )),
            }
        }
        _ => Err(field_error(
            field_name,
            "Geolocation must be a GeoJSON object or a \"lat,lon\" string",
        )),
    }
}

/// The `GeoJSON` geometry type names (the ones that carry a `coordinates`
/// array, and the only ones a Feature's `geometry` may be).
const GEOJSON_GEOMETRIES: [&str; 7] = [
    "Point",
    "MultiPoint",
    "LineString",
    "MultiLineString",
    "Polygon",
    "MultiPolygon",
    "GeometryCollection",
];

/// Structural `GeoJSON` check: a known `type` plus the member that type
/// requires (`coordinates` for geometries, `geometry` for a Feature — which
/// may be null, the way Frappe leaves an empty one — and the corresponding
/// arrays for the collections).
fn validate_geojson_shape(field_name: &str, geo: &Value) -> Result<()> {
    let invalid = |message: &str| field_error(field_name, message);
    let Some(kind) = geo.get("type").and_then(Value::as_str) else {
        return Err(invalid("GeoJSON must declare a \"type\""));
    };
    match kind {
        "Feature" => {
            if let Some(geometry) = geo.get("geometry")
                && !geometry.is_null()
            {
                let geometry_kind = geometry.get("type").and_then(Value::as_str);
                if geometry_kind.is_none_or(|g| !GEOJSON_GEOMETRIES.contains(&g)) {
                    return Err(invalid("A GeoJSON Feature's geometry must be a geometry"));
                }
            }
            Ok(())
        }
        "FeatureCollection" => require_member_array(field_name, geo, kind, "features"),
        "GeometryCollection" => require_member_array(field_name, geo, kind, "geometries"),
        g if GEOJSON_GEOMETRIES.contains(&g) => {
            require_member_array(field_name, geo, g, "coordinates")
        }
        _ => Err(invalid(&format!("Unknown GeoJSON type '{kind}'"))),
    }
}

/// Every `GeoJSON` member a root or nested node may require must be an array.
fn require_member_array(field_name: &str, geo: &Value, kind: &str, member: &str) -> Result<()> {
    if geo.get(member).and_then(Value::as_array).is_some() {
        Ok(())
    } else {
        Err(field_error(
            field_name,
            format!("A GeoJSON '{kind}' must carry a '{member}' array"),
        ))
    }
}

/// The legacy `"lat,lon"` pair, range-checked the same way the Desk's map
/// widget plots it.
fn latlon_in_range(lat: &str, lon: &str) -> bool {
    match (lat.trim().parse::<f64>(), lon.trim().parse::<f64>()) {
        (Ok(lat), Ok(lon)) => (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon),
        _ => false,
    }
}

/// `Signature` is the canvas' `toDataURL` output — a base64 data URL, the
/// same shape Frappe stores. A non-string or a non-data-URL string would
/// render as a broken `<img>` later, so it is refused at the edge; the
/// base64 alphabet itself is the browser's to enforce.
pub(super) fn validate_signature(field_name: &str, value: &Value) -> Result<()> {
    let Some(text) = value.as_str() else {
        return Err(field_error(
            field_name,
            "Signature must be a data URL string",
        ));
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    let valid = text
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .is_some_and(|(_mime, payload)| !payload.is_empty());
    if !valid {
        return Err(field_error(
            field_name,
            "Signature must be a base64 data URL (data:<mime>;base64,…)",
        ));
    }
    Ok(())
}

/// `Attach`/`Attach Image` store the file's served path or name as a string;
/// the gallery and `<img>` renderers treat it as text and nothing else, so a
/// non-string storage shape (object, array, number) is refused here. The
/// path's own reachability is checked by the upload/download endpoints, not
/// by field validation.
pub(super) fn validate_attach(field_name: &str, value: &Value) -> Result<()> {
    if value.as_str().is_none() {
        return Err(field_error(
            field_name,
            "Attachment value must be a file path string",
        ));
    }
    Ok(())
}

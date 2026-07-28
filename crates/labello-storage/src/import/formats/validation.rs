fn slug(value: &str) -> String {
    let mut output = String::new();
    let mut dash = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
            dash = false;
        } else if !output.is_empty() && !dash {
            output.push('-');
            dash = true;
        }
    }
    output.trim_end_matches('-').chars().take(80).collect()
}

fn validate_or_clip_box(
    bbox: &mut F64Box,
    policy: GeometryBoundsPolicy,
    diagnostics: &mut Diagnostics,
    path: &str,
    line: usize,
) -> StorageResult<bool> {
    if !bbox.x.is_finite()
        || !bbox.y.is_finite()
        || !bbox.width.is_finite()
        || !bbox.height.is_finite()
        || bbox.width <= 0.0
        || bbox.height <= 0.0
    {
        return Err(import_error(
            "geometry_invalid",
            "bounding box values must be finite and positive",
        ));
    }
    let valid =
        bbox.x >= 0.0 && bbox.y >= 0.0 && bbox.x + bbox.width <= 1.0 && bbox.y + bbox.height <= 1.0;
    if valid {
        return Ok(false);
    }
    if policy == GeometryBoundsPolicy::Block {
        return Err(import_error(
            "geometry_out_of_bounds",
            "bounding box crosses decoded image bounds",
        ));
    }
    let right = (bbox.x + bbox.width).clamp(0.0, 1.0);
    let bottom = (bbox.y + bbox.height).clamp(0.0, 1.0);
    bbox.x = bbox.x.clamp(0.0, 1.0);
    bbox.y = bbox.y.clamp(0.0, 1.0);
    bbox.width = right - bbox.x;
    bbox.height = bottom - bbox.y;
    if bbox.width <= 0.0 || bbox.height <= 0.0 {
        return Err(import_error(
            "geometry_clip_empty",
            "clipping produced an empty bounding box",
        ));
    }
    diagnostics.add(
        "geometry_clipped",
        DiagnosticSeverity::WarningRequiresAck,
        "out-of-bounds box is clipped and marked derived",
        false,
        true,
        true,
        Some(if line > 0 {
            example_line(path, line)
        } else {
            example_path(path)
        }),
    );
    Ok(true)
}

fn normalize_yolo_bbox_boundary(
    center_x: f64,
    center_y: f64,
    width: f64,
    height: f64,
) -> (F64Box, bool) {
    let left = center_x - width / 2.0;
    let right = center_x + width / 2.0;
    let top = center_y - height / 2.0;
    let bottom = center_y + height / 2.0;
    let reconstructed_right = left + width;
    let reconstructed_bottom = top + height;
    let outside =
        left < 0.0 || top < 0.0 || reconstructed_right > 1.0 || reconstructed_bottom > 1.0;
    let comparison_margin = f64::EPSILON * 8.0;
    let lower_bound = -YOLO_BOUNDARY_ROUNDING_TOLERANCE - comparison_margin;
    let upper_bound = 1.0 + YOLO_BOUNDARY_ROUNDING_TOLERANCE + comparison_margin;
    let within_rounding_tolerance = left >= lower_bound
        && top >= lower_bound
        && right <= upper_bound
        && bottom <= upper_bound
        && reconstructed_right <= upper_bound
        && reconstructed_bottom <= upper_bound;
    if outside && within_rounding_tolerance {
        let left = left.clamp(0.0, 1.0);
        let right = right.clamp(0.0, 1.0);
        let top = top.clamp(0.0, 1.0);
        let bottom = bottom.clamp(0.0, 1.0);
        return (
            F64Box {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            },
            true,
        );
    }
    (
        F64Box {
            x: left,
            y: top,
            width,
            height,
        },
        false,
    )
}

fn yaml_strings(value: &Value) -> StorageResult<Vec<String>> {
    let values = match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(values) if !values.is_empty() => values
            .iter()
            .map(|value| {
                value.as_str().map(str::to_string).ok_or_else(|| {
                    import_error("yolo_split_invalid", "YOLO split entries must be strings")
                })
            })
            .collect::<StorageResult<Vec<_>>>()?,
        _ => Err(import_error(
            "yolo_split_invalid",
            "YOLO split must be a nonempty string or list of strings",
        ))?,
    };
    if values.iter().any(|value| value.trim().is_empty()) {
        return Err(import_error(
            "yolo_split_invalid",
            "YOLO split paths must be nonempty strings",
        ));
    }
    Ok(values)
}

fn required_array<'a>(
    root: &'a serde_json::Map<String, Value>,
    key: &str,
) -> StorageResult<&'a Vec<Value>> {
    root.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| import_error("coco_field_invalid", format!("COCO {key} must be an array")))
}
fn required_string<'a>(
    root: &'a serde_json::Map<String, Value>,
    key: &str,
    code: &str,
) -> StorageResult<&'a str> {
    root.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| import_error(code, format!("COCO {key} must be a nonempty string")))
}
fn json_id(value: Option<&Value>, code: &str) -> StorageResult<u64> {
    value
        .and_then(Value::as_u64)
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or_else(|| import_error(code, "COCO IDs must be JSON integers in 0..=i64::MAX"))
}
fn json_u32(value: Option<&Value>, code: &str) -> StorageResult<u32> {
    json_id(value, code)
        .and_then(|value| {
            u32::try_from(value).map_err(|_| import_error(code, "COCO dimension exceeds u32"))
        })
        .and_then(|value| {
            if value == 0 {
                Err(import_error(code, "COCO dimensions must be positive"))
            } else {
                Ok(value)
            }
        })
}
fn finite_array(value: Option<&Value>, length: usize, code: &str) -> StorageResult<Vec<f64>> {
    let values = value
        .and_then(Value::as_array)
        .filter(|values| values.len() == length)
        .ok_or_else(|| {
            import_error(
                code,
                format!("numeric array must contain exactly {length} values"),
            )
        })?;
    values
        .iter()
        .map(|value| {
            value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| import_error(code, "numeric arrays require finite JSON numbers"))
        })
        .collect()
}
fn parse_integer_token(value: &str, code: &str) -> StorageResult<u64> {
    if value.contains(['.', 'e', 'E']) {
        return Err(import_error(code, "class index must be an integer token"));
    }
    value
        .parse()
        .map_err(|_| import_error(code, "class index must be a non-negative integer"))
}
fn parse_finite(value: &str, code: &str) -> StorageResult<f64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| import_error(code, "numeric token must be finite"))
}
fn is_image_path(path: &str) -> bool {
    matches!(
        source_extension(path).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
    )
}
fn below(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}
pub(super) fn coverage_key(image: &str, category: &str) -> String {
    format!("{image}\0{category}")
}

fn enforce_json_nesting(bytes: &[u8], limit: usize) -> StorageResult<()> {
    let mut depth = 0_usize;
    let mut string = false;
    let mut escape = false;
    for byte in bytes {
        if string {
            if escape {
                escape = false;
            } else if *byte == b'\\' {
                escape = true;
            } else if *byte == b'"' {
                string = false;
            }
            continue;
        }
        match byte {
            b'"' => string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > limit {
                    return Err(import_error(
                        "json_nesting_limit",
                        "JSON nesting exceeds configured limit",
                    ));
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

fn validate_yaml_value(
    value: &serde_yaml_ng::Value,
    limit: usize,
    depth: usize,
    nodes: &mut usize,
) -> StorageResult<()> {
    *nodes = nodes.saturating_add(1);
    if *nodes > MAX_STRUCTURED_NODES {
        return Err(import_error(
            "structured_data_node_limit",
            "structured data exceeds the parser node limit",
        ));
    }
    if depth > limit {
        return Err(import_error(
            "yolo_yaml_nesting",
            "YAML nesting exceeds the configured limit",
        ));
    }
    match value {
        serde_yaml_ng::Value::Tagged(_) => Err(import_error(
            "yolo_yaml_tag_rejected",
            "custom YAML tags are not supported",
        )),
        serde_yaml_ng::Value::Sequence(values) => {
            for value in values {
                validate_yaml_value(value, limit, depth + 1, nodes)?;
            }
            Ok(())
        }
        serde_yaml_ng::Value::Mapping(values) => {
            for (key, value) in values {
                validate_yaml_value(key, limit, depth + 1, nodes)?;
                validate_yaml_value(value, limit, depth + 1, nodes)?;
            }
            Ok(())
        }
        serde_yaml_ng::Value::String(value) if value.len() > MAX_STRUCTURED_VALUE_BYTES => {
            Err(import_error(
                "structured_data_value_limit",
                "structured data contains an oversized scalar value",
            ))
        }
        _ => Ok(()),
    }
}

fn enforce_yaml_alias_limit(bytes: &[u8], limit: usize) -> StorageResult<()> {
    let mut aliases = 0_usize;
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let mut comment = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if comment {
            if byte == b'\n' {
                comment = false;
            }
            continue;
        }
        if double_quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                double_quoted = false;
            }
            continue;
        }
        if single_quoted {
            if byte == b'\'' {
                if bytes.get(index + 1) == Some(&b'\'') || (index > 0 && bytes[index - 1] == b'\'')
                {
                    continue;
                }
                single_quoted = false;
            }
            continue;
        }
        match byte {
            b'#' => comment = true,
            b'"' => double_quoted = true,
            b'\'' => single_quoted = true,
            b'*' | b'&'
                if index == 0
                    || bytes[index - 1].is_ascii_whitespace()
                    || matches!(bytes[index - 1], b'[' | b'{' | b',' | b':' | b'?' | b'-') =>
            {
                aliases += 1;
                if aliases > limit {
                    return Err(import_error(
                        "yolo_yaml_alias_limit",
                        "YAML anchors and aliases exceed the parser alias limit",
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_json_value(value: &Value, nodes: &mut usize) -> StorageResult<()> {
    *nodes = nodes.saturating_add(1);
    if *nodes > MAX_STRUCTURED_NODES {
        return Err(import_error(
            "structured_data_node_limit",
            "structured data exceeds the parser node limit",
        ));
    }
    match value {
        Value::String(value) if value.len() > MAX_STRUCTURED_VALUE_BYTES => Err(import_error(
            "structured_data_value_limit",
            "structured data contains an oversized scalar value",
        )),
        Value::Array(values) => {
            for value in values {
                validate_json_value(value, nodes)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, value) in values {
                if key.len() > MAX_STRUCTURED_VALUE_BYTES {
                    return Err(import_error(
                        "structured_data_value_limit",
                        "structured data contains an oversized mapping key",
                    ));
                }
                validate_json_value(value, nodes)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn for_each_bounded_line(
    path: &std::path::Path,
    max_line_bytes: usize,
    mut visit: impl FnMut(usize, &str) -> StorageResult<()>,
) -> StorageResult<()> {
    let file = std::fs::File::open(path).map_err(|source| StorageError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut reader = std::io::BufReader::new(file);
    let mut line = Vec::with_capacity(max_line_bytes.min(16 * 1024));
    let mut line_number = 0_usize;
    loop {
        line.clear();
        let mut limited = (&mut reader).take(max_line_bytes.saturating_add(1) as u64);
        let read = BufRead::read_until(&mut limited, b'\n', &mut line).map_err(|source| {
            StorageError::Io {
                path: path.to_path_buf(),
                source,
            }
        })?;
        if read == 0 {
            break;
        }
        line_number += 1;
        while line
            .last()
            .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
        {
            line.pop();
        }
        if line.len() > max_line_bytes {
            return Err(import_error(
                "yolo_line_limit",
                "YOLO label line exceeds the configured limit",
            ));
        }
        let text = std::str::from_utf8(&line)
            .map_err(|_| import_error("yolo_label_utf8", "YOLO label file must be UTF-8"))?;
        visit(line_number, text)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn free_space_bytes(path: &std::path::Path) -> Option<u64> {
    rustix::fs::statvfs(path)
        .ok()
        .and_then(|value| value.f_bavail.checked_mul(value.f_frsize))
}

#[cfg(not(target_os = "linux"))]
fn free_space_bytes(_path: &std::path::Path) -> Option<u64> {
    None
}

fn enforce_value_depth(value: &Value, limit: usize, code: &str) -> StorageResult<()> {
    fn depth(value: &Value) -> usize {
        match value {
            Value::Array(values) => 1 + values.iter().map(depth).max().unwrap_or(0),
            Value::Object(values) => 1 + values.values().map(depth).max().unwrap_or(0),
            _ => 1,
        }
    }
    if depth(value) > limit {
        Err(import_error(
            code,
            "structured-data nesting exceeds configured limit",
        ))
    } else {
        Ok(())
    }
}

fn example_path(path: &str) -> DiagnosticExample {
    DiagnosticExample {
        source_path: Some(path.to_string()),
        source_image_key: None,
        source_object_key: None,
        line: None,
    }
}
fn example_line(path: &str, line: usize) -> DiagnosticExample {
    DiagnosticExample {
        source_path: Some(path.to_string()),
        source_image_key: None,
        source_object_key: None,
        line: Some(line as u64),
    }
}
fn example_object(path: &str, id: u64) -> DiagnosticExample {
    DiagnosticExample {
        source_path: Some(path.to_string()),
        source_image_key: None,
        source_object_key: Some(id.to_string()),
        line: None,
    }
}

struct Diagnostics {
    profile: ImportProfile,
    example_limit: usize,
    values: BTreeMap<String, ImportDiagnostic>,
}
impl Diagnostics {
    fn new(profile: ImportProfile, example_limit: usize) -> Self {
        Self {
            profile,
            example_limit,
            values: BTreeMap::new(),
        }
    }
    // These flags are the persisted `ImportDiagnostic` dimensions. A builder
    // would only rename the same arguments at every current parser call site.
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        code: &str,
        severity: DiagnosticSeverity,
        summary: &str,
        blocks: bool,
        ack: bool,
        coverage: bool,
        example: Option<DiagnosticExample>,
    ) {
        let value = self
            .values
            .entry(code.to_string())
            .or_insert_with(|| ImportDiagnostic {
                code: code.to_string(),
                severity,
                profile: self.profile,
                count: 0,
                summary: summary.to_string(),
                blocks_commit: blocks,
                requires_acknowledgement: ack,
                changes_coverage: coverage,
                examples: Vec::new(),
            });
        value.count += 1;
        if let Some(example) = example
            && value.examples.len() < self.example_limit
        {
            value.examples.push(example);
        }
    }
    // Keep the counted and single-occurrence forms visibly symmetric.
    #[allow(clippy::too_many_arguments)]
    fn add_count(
        &mut self,
        code: &str,
        severity: DiagnosticSeverity,
        summary: &str,
        count: u64,
        blocks: bool,
        ack: bool,
        coverage: bool,
    ) {
        self.add(code, severity, summary, blocks, ack, coverage, None);
        self.values.get_mut(code).unwrap().count = count;
    }
    fn finish(self) -> Vec<ImportDiagnostic> {
        self.values.into_values().collect()
    }
}

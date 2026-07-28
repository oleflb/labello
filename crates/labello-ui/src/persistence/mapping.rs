fn stored_view(view: crate::app::AppView) -> StoredView {
    match view {
        crate::app::AppView::Annotate => StoredView::Annotate,
        crate::app::AppView::Review => StoredView::Review,
        crate::app::AppView::Adjudicate => StoredView::Adjudicate,
        crate::app::AppView::Admin => StoredView::Admin,
        crate::app::AppView::Stats | crate::app::AppView::Setup => StoredView::Stats,
    }
}

fn app_view(view: StoredView) -> crate::app::AppView {
    match view {
        StoredView::Annotate => crate::app::AppView::Annotate,
        StoredView::Review => crate::app::AppView::Review,
        StoredView::Adjudicate => crate::app::AppView::Adjudicate,
        StoredView::Admin => crate::app::AppView::Admin,
        StoredView::Stats => crate::app::AppView::Stats,
    }
}

fn assignment_kind_segment(kind: &AssignmentKind) -> &'static str {
    match kind {
        AssignmentKind::Annotation => "annotation",
        AssignmentKind::Review => "review",
        AssignmentKind::Adjudication => "adjudication",
    }
}

fn key_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn same_work_draft(left: &WorkDraft, right: &WorkDraft) -> bool {
    left.key == right.key
        && left.base_event_sequence == right.base_event_sequence
        && left.edit_generation == right.edit_generation
        && left.payload == right.payload
}

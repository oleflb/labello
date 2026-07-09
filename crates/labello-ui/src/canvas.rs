use egui::{
    Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, WidgetInfo,
    WidgetType, pos2, vec2,
};
use labello_domain::{AnnotationGeometry, AnnotationId, AnnotationVersion, BoundingBox};

#[derive(Clone, Debug, PartialEq)]
pub enum CanvasAction {
    CreateBoundingBox(BoundingBox),
    Select(AnnotationId),
}

#[derive(Clone, Debug, Default)]
pub struct CanvasState {
    drag_start: Option<Pos2>,
    draft_box: Option<BoundingBox>,
}

pub fn show_canvas(
    ui: &mut Ui,
    state: &mut CanvasState,
    texture: Option<&egui::TextureHandle>,
    annotations: &[AnnotationVersion],
    image_size: [u32; 2],
    bounding_box_tool: bool,
) -> Option<CanvasAction> {
    let available = ui.available_size().max(vec2(300.0, 240.0));
    let image_ratio = image_size[0].max(1) as f32 / image_size[1].max(1) as f32;
    let mut canvas_size = available;
    if canvas_size.x / canvas_size.y > image_ratio {
        canvas_size.x = canvas_size.y * image_ratio;
    } else {
        canvas_size.y = canvas_size.x / image_ratio;
    }
    let (rect, response) = ui.allocate_exact_size(canvas_size, Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Annotation canvas"));
    paint_canvas(ui, rect, texture, annotations, state.draft_box);
    handle_pointer(response, rect, state, annotations, bounding_box_tool)
}

fn paint_canvas(
    ui: &Ui,
    rect: Rect,
    texture: Option<&egui::TextureHandle>,
    annotations: &[AnnotationVersion],
    draft_box: Option<BoundingBox>,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(18), Color32::from_rgb(18, 23, 34));
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    painter.rect_stroke(
        rect,
        CornerRadius::same(18),
        Stroke::new(1.0, Color32::from_rgb(70, 82, 105)),
        StrokeKind::Inside,
    );

    let grid = Color32::from_rgba_unmultiplied(255, 255, 255, 18);
    for i in 1..4 {
        let x = rect.left() + rect.width() * i as f32 / 4.0;
        let y = rect.top() + rect.height() * i as f32 / 4.0;
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.0, grid),
        );
        painter.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(1.0, grid),
        );
    }

    for annotation in annotations.iter().filter(|annotation| !annotation.deleted) {
        match &annotation.geometry {
            AnnotationGeometry::BoundingBox(bbox) => {
                paint_box(ui, rect, *bbox, Color32::from_rgb(94, 234, 212), 2.0)
            }
            AnnotationGeometry::Skeleton(skeleton) => {
                for keypoint in &skeleton.keypoints {
                    if let Some(point) = keypoint.point {
                        let center = pos2(
                            rect.left() + point.x * rect.width(),
                            rect.top() + point.y * rect.height(),
                        );
                        painter.circle_filled(center, 4.0, Color32::from_rgb(250, 204, 21));
                    }
                }
            }
        }
    }
    if let Some(bbox) = draft_box {
        paint_box(ui, rect, bbox, Color32::from_rgb(96, 165, 250), 1.5);
    }
}

fn paint_box(ui: &Ui, image_rect: Rect, bbox: BoundingBox, color: Color32, width: f32) {
    let painter = ui.painter_at(image_rect);
    let rect = Rect::from_min_size(
        pos2(
            image_rect.left() + bbox.x * image_rect.width(),
            image_rect.top() + bbox.y * image_rect.height(),
        ),
        vec2(
            bbox.width * image_rect.width(),
            bbox.height * image_rect.height(),
        ),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(5),
        Stroke::new(width, color),
        StrokeKind::Inside,
    );
    painter.rect_filled(
        Rect::from_min_size(rect.min, vec2(rect.width(), 20.0)),
        CornerRadius::same(5),
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 48),
    );
}

fn handle_pointer(
    response: Response,
    rect: Rect,
    state: &mut CanvasState,
    annotations: &[AnnotationVersion],
    bounding_box_tool: bool,
) -> Option<CanvasAction> {
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(annotation) = annotation_at(pos, rect, annotations)
    {
        return Some(CanvasAction::Select(annotation.annotation_id.clone()));
    }
    if !bounding_box_tool {
        return None;
    }
    let pointer_down_on_canvas = response.is_pointer_button_down_on();
    if state.drag_start.is_none() && (response.drag_started() || pointer_down_on_canvas) {
        state.drag_start = response.interact_pointer_pos();
        state.draft_box = None;
    }
    if (response.dragged() || pointer_down_on_canvas)
        && let (Some(start), Some(current)) = (state.drag_start, response.interact_pointer_pos())
    {
        state.draft_box = bbox_from_points(rect, start, current);
    }
    let released = response.ctx.input(|input| input.pointer.any_released());
    if released {
        state.drag_start = None;
        if let Some(bbox) = state.draft_box.take()
            && bbox.width > 0.005
            && bbox.height > 0.005
        {
            return Some(CanvasAction::CreateBoundingBox(bbox));
        }
    }
    None
}

fn bbox_from_points(rect: Rect, a: Pos2, b: Pos2) -> Option<BoundingBox> {
    let min = pos2(
        a.x.min(b.x).clamp(rect.left(), rect.right()),
        a.y.min(b.y).clamp(rect.top(), rect.bottom()),
    );
    let max = pos2(
        a.x.max(b.x).clamp(rect.left(), rect.right()),
        a.y.max(b.y).clamp(rect.top(), rect.bottom()),
    );
    let bbox = BoundingBox {
        x: (min.x - rect.left()) / rect.width(),
        y: (min.y - rect.top()) / rect.height(),
        width: (max.x - min.x) / rect.width(),
        height: (max.y - min.y) / rect.height(),
    };
    bbox.validate().ok().map(|_| bbox)
}

fn annotation_at(
    pos: Pos2,
    rect: Rect,
    annotations: &[AnnotationVersion],
) -> Option<&AnnotationVersion> {
    annotations
        .iter()
        .rev()
        .find(|annotation| match annotation.geometry {
            AnnotationGeometry::BoundingBox(bbox) => Rect::from_min_size(
                pos2(
                    rect.left() + bbox.x * rect.width(),
                    rect.top() + bbox.y * rect.height(),
                ),
                vec2(bbox.width * rect.width(), bbox.height * rect.height()),
            )
            .contains(pos),
            AnnotationGeometry::Skeleton(_) => false,
        })
}

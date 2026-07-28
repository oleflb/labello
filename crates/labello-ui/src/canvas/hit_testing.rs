fn keypoint_at(
    pos: Pos2,
    image_rect: Rect,
    annotation_id: &AnnotationId,
    annotations: &[AnnotationVersion],
) -> Option<(usize, NormalizedPoint)> {
    let annotation = annotations
        .iter()
        .rev()
        .find(|annotation| !annotation.deleted && &annotation.annotation_id == annotation_id)?;
    let AnnotationGeometry::Skeleton(skeleton) = &annotation.geometry else {
        return None;
    };
    skeleton
        .keypoints
        .iter()
        .enumerate()
        .filter_map(|(index, keypoint)| keypoint.point.map(|point| (index, point)))
        .find(|(_, point)| {
            normalized_to_screen(image_rect, pos2(point.x, point.y)).distance(pos) <= 12.0
        })
}

fn annotation_bbox(id: &AnnotationId, annotations: &[AnnotationVersion]) -> Option<BoundingBox> {
    annotations.iter().rev().find_map(|annotation| {
        if !annotation.deleted && &annotation.annotation_id == id {
            match annotation.geometry {
                AnnotationGeometry::BoundingBox(bbox) => Some(clamp_bbox(bbox)),
                AnnotationGeometry::Skeleton(_) => None,
            }
        } else {
            None
        }
    })
}

fn annotation_at(
    pos: Pos2,
    image_rect: Rect,
    annotations: &[AnnotationVersion],
) -> Option<&AnnotationVersion> {
    annotations
        .iter()
        .rev()
        .filter(|annotation| !annotation.deleted)
        .find(|annotation| match annotation.geometry {
            AnnotationGeometry::BoundingBox(bbox) => {
                bbox_to_screen_rect(image_rect, bbox).contains(pos)
            }
            AnnotationGeometry::Skeleton(ref skeleton) => {
                skeleton.keypoints.iter().any(|keypoint| {
                    keypoint.point.is_some_and(|point| {
                        normalized_to_screen(image_rect, pos2(point.x, point.y)).distance(pos)
                            <= 12.0
                    })
                })
            }
        })
}

fn bbox_from_normalized_points(a: Pos2, b: Pos2) -> Option<BoundingBox> {
    let min = pos2(a.x.min(b.x).clamp(0.0, 1.0), a.y.min(b.y).clamp(0.0, 1.0));
    let max = pos2(a.x.max(b.x).clamp(0.0, 1.0), a.y.max(b.y).clamp(0.0, 1.0));
    let bbox = BoundingBox {
        x: min.x,
        y: min.y,
        width: max.x - min.x,
        height: max.y - min.y,
    };
    (bbox.width > 0.0 && bbox.height > 0.0).then_some(bbox)
}

fn clamp_bbox(bbox: BoundingBox) -> BoundingBox {
    let x1 = finite_or(bbox.x, 0.0).clamp(0.0, 1.0 - MIN_BOX_SIZE);
    let y1 = finite_or(bbox.y, 0.0).clamp(0.0, 1.0 - MIN_BOX_SIZE);
    let x2 = finite_or(bbox.x + bbox.width, x1 + MIN_BOX_SIZE).clamp(x1 + MIN_BOX_SIZE, 1.0);
    let y2 = finite_or(bbox.y + bbox.height, y1 + MIN_BOX_SIZE).clamp(y1 + MIN_BOX_SIZE, 1.0);
    BoundingBox {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn move_bbox(bbox: BoundingBox, delta: Vec2) -> BoundingBox {
    let bbox = clamp_bbox(bbox);
    BoundingBox {
        x: (bbox.x + finite_or(delta.x, 0.0)).clamp(0.0, 1.0 - bbox.width),
        y: (bbox.y + finite_or(delta.y, 0.0)).clamp(0.0, 1.0 - bbox.height),
        ..bbox
    }
}

fn bbox_changed(a: BoundingBox, b: BoundingBox) -> bool {
    (a.x - b.x).abs() > f32::EPSILON
        || (a.y - b.y).abs() > f32::EPSILON
        || (a.width - b.width).abs() > f32::EPSILON
        || (a.height - b.height).abs() > f32::EPSILON
}

fn resize_bbox(bbox: BoundingBox, handle: ResizeHandle, pointer: Pos2) -> BoundingBox {
    let bbox = clamp_bbox(bbox);
    let mut left = bbox.x;
    let mut top = bbox.y;
    let mut right = bbox.x + bbox.width;
    let mut bottom = bbox.y + bbox.height;
    let pointer = pos2(finite_or(pointer.x, left), finite_or(pointer.y, top));

    if matches!(
        handle,
        ResizeHandle::TopLeft | ResizeHandle::Left | ResizeHandle::BottomLeft
    ) {
        left = pointer.x.clamp(0.0, right - MIN_BOX_SIZE);
    }
    if matches!(
        handle,
        ResizeHandle::TopRight | ResizeHandle::Right | ResizeHandle::BottomRight
    ) {
        right = pointer.x.clamp(left + MIN_BOX_SIZE, 1.0);
    }
    if matches!(
        handle,
        ResizeHandle::TopLeft | ResizeHandle::Top | ResizeHandle::TopRight
    ) {
        top = pointer.y.clamp(0.0, bottom - MIN_BOX_SIZE);
    }
    if matches!(
        handle,
        ResizeHandle::BottomLeft | ResizeHandle::Bottom | ResizeHandle::BottomRight
    ) {
        bottom = pointer.y.clamp(top + MIN_BOX_SIZE, 1.0);
    }

    BoundingBox {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

fn resize_handles(rect: Rect) -> [(ResizeHandle, Pos2); 8] {
    [
        (ResizeHandle::TopLeft, rect.left_top()),
        (ResizeHandle::Top, pos2(rect.center().x, rect.top())),
        (ResizeHandle::TopRight, rect.right_top()),
        (ResizeHandle::Right, pos2(rect.right(), rect.center().y)),
        (ResizeHandle::BottomRight, rect.right_bottom()),
        (ResizeHandle::Bottom, pos2(rect.center().x, rect.bottom())),
        (ResizeHandle::BottomLeft, rect.left_bottom()),
        (ResizeHandle::Left, pos2(rect.left(), rect.center().y)),
    ]
}

fn resize_handle_at(pointer: Pos2, rect: Rect) -> Option<ResizeHandle> {
    resize_handles(rect)
        .into_iter()
        .find_map(|(handle, center)| {
            (center.distance(pointer) <= HANDLE_HIT_RADIUS).then_some(handle)
        })
}

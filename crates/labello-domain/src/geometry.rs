use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{DomainError, DomainResult};

const EPSILON: f32 = 0.000_001;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageDimensions {
    pub width: u32,
    pub height: u32,
}

impl ImageDimensions {
    pub fn validate(self) -> DomainResult<()> {
        if self.width == 0 || self.height == 0 {
            return Err(DomainError::InvalidGeometry(
                "image dimensions must be non-zero".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedPoint {
    pub x: f32,
    pub y: f32,
}

impl NormalizedPoint {
    pub fn validate(self) -> DomainResult<()> {
        validate_unit("x", self.x)?;
        validate_unit("y", self.y)?;
        Ok(())
    }

    pub fn distance(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx.mul_add(dx, dy * dy)).sqrt()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoundingBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl BoundingBox {
    pub fn validate(self) -> DomainResult<()> {
        validate_unit("x", self.x)?;
        validate_unit("y", self.y)?;
        if self.width <= 0.0 || self.height <= 0.0 {
            return Err(DomainError::InvalidGeometry(
                "bounding box width and height must be positive".to_string(),
            ));
        }
        validate_unit("width", self.width)?;
        validate_unit("height", self.height)?;
        if self.x + self.width > 1.0 + EPSILON || self.y + self.height > 1.0 + EPSILON {
            return Err(DomainError::InvalidGeometry(
                "bounding box must fit within normalized image bounds".to_string(),
            ));
        }
        Ok(())
    }

    pub fn area(self) -> f32 {
        self.width.max(0.0) * self.height.max(0.0)
    }

    pub fn iou(self, other: Self) -> f32 {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        let intersection = (right - left).max(0.0) * (bottom - top).max(0.0);
        let union = self.area() + other.area() - intersection;
        if union <= 0.0 {
            0.0
        } else {
            intersection / union
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KeypointState {
    Visible,
    Hidden,
    Absent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KeypointAnnotation {
    pub name: String,
    pub state: KeypointState,
    pub point: Option<NormalizedPoint>,
}

impl KeypointAnnotation {
    pub fn validate(&self) -> DomainResult<()> {
        match (&self.state, self.point) {
            (KeypointState::Absent, Some(_)) => Err(DomainError::InvalidGeometry(format!(
                "absent keypoint {} cannot have coordinates",
                self.name
            ))),
            (KeypointState::Visible | KeypointState::Hidden, None) => {
                Err(DomainError::InvalidGeometry(format!(
                    "keypoint {} requires normalized coordinates",
                    self.name
                )))
            }
            (_, Some(point)) => point.validate(),
            (_, None) => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkeletonGeometry {
    pub keypoints: Vec<KeypointAnnotation>,
}

impl SkeletonGeometry {
    pub fn validate(&self) -> DomainResult<()> {
        for keypoint in &self.keypoints {
            keypoint.validate()?;
        }
        Ok(())
    }

    pub fn mean_distance(&self, other: &Self) -> Option<f32> {
        let mut total = 0.0;
        let mut count = 0;
        for a in &self.keypoints {
            let Some(a_point) = a.point else { continue };
            let Some(b) = other
                .keypoints
                .iter()
                .find(|candidate| candidate.name == a.name)
            else {
                continue;
            };
            let Some(b_point) = b.point else { continue };
            total += a_point.distance(b_point);
            count += 1;
        }
        (count > 0).then_some(total / count as f32)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "geometry", rename_all = "snake_case")]
pub enum AnnotationGeometry {
    BoundingBox(BoundingBox),
    Skeleton(SkeletonGeometry),
}

impl AnnotationGeometry {
    pub fn validate(&self) -> DomainResult<()> {
        match self {
            Self::BoundingBox(bbox) => bbox.validate(),
            Self::Skeleton(skeleton) => skeleton.validate(),
        }
    }
}

fn validate_unit(name: &str, value: f32) -> DomainResult<()> {
    if !value.is_finite() || !(0.0 - EPSILON..=1.0 + EPSILON).contains(&value) {
        return Err(DomainError::InvalidGeometry(format!(
            "{name} must be a finite normalized value in [0, 1]"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_normalized_bounds() {
        assert!(
            BoundingBox {
                x: 0.1,
                y: 0.2,
                width: 0.3,
                height: 0.4
            }
            .validate()
            .is_ok()
        );
        assert!(
            BoundingBox {
                x: 0.8,
                y: 0.2,
                width: 0.3,
                height: 0.4
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn computes_iou() {
        let a = BoundingBox {
            x: 0.0,
            y: 0.0,
            width: 0.5,
            height: 0.5,
        };
        let b = BoundingBox {
            x: 0.25,
            y: 0.25,
            width: 0.5,
            height: 0.5,
        };
        let iou = a.iou(b);
        assert!((iou - 1.0 / 7.0).abs() < 0.0001);
    }
}

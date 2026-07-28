use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use labello_domain::{
    ClassId, DatasetId, ImportCoverageTotals, ImportDescriptorKind, ImportGeometryMapping,
    ImportId, TaskDefinition, Timestamp, UserId,
};
use serde::{Deserialize, Serialize};

include!("types/capabilities.rs");
include!("types/jobs.rs");
include!("types/planning.rs");
include!("types/publication.rs");

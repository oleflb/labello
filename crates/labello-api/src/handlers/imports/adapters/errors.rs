fn map_storage(error: storage::StorageError) -> ApiError {
    match error {
        storage::StorageError::NotFound(_) => ApiError::NotFound("import job".to_string()),
        storage::StorageError::Import { code, message } => match code.as_str() {
            "import_owner_mismatch" => ApiError::NotFound("import job".to_string()),
            "import_root_forbidden" => ApiError::Forbidden("import root access denied".to_string()),
            "import_id_invalid" | "destination_id_invalid" | "destination_id_reserved" => {
                ApiError::BadRequest(message)
            }
            "source_file_limit"
            | "source_byte_limit"
            | "source_file_too_large"
            | "server_source_browse_limit"
            | "upload_chunk_limit"
            | "selected_image_limit"
            | "annotation_limit"
            | "keypoint_limit" => ApiError::PayloadTooLarge(message),
            "destination_exists"
            | "destination_reserved"
            | "source_path_collision"
            | "job_phase_invalid"
            | "source_sealed"
            | "upload_chunk_not_sequential"
            | "upload_chunk_retry_mismatch"
            | "source_changed"
            | "plan_stale"
            | "job_not_cancellable"
            | "reservation_limit"
            | "upload_concurrency_limit"
            | "build_concurrency_limit"
            | "descriptor_inspection_busy"
            | "import_unavailable" => ApiError::Conflict(message),
            "profile_disabled"
            | "destination_name_invalid"
            | "source_incomplete"
            | "ground_truth_attestation_required"
            | "plan_not_committable"
            | "parser_time_limit"
            | "source_file_missing"
            | "import_root_missing"
            | "upload_chunk_digest_mismatch"
            | "source_file_digest_mismatch" => ApiError::Unprocessable(message),
            _ if code.starts_with("yolo_")
                || code.starts_with("coco_")
                || code.starts_with("image_")
                || code.starts_with("descriptor_")
                || code.starts_with("server_source_")
                || code.starts_with("source_path_")
                || code.starts_with("source_file_")
                || code.starts_with("geometry_")
                || code.starts_with("category_") =>
            {
                ApiError::Unprocessable(message)
            }
            _ => ApiError::Storage(storage::StorageError::Import { code, message }),
        },
        error => ApiError::Storage(error),
    }
}

impl LabelloApp {
    fn dispatch_support_command(
        &self,
        api: Rc<dyn labello_client::LabelloApi>,
        command: UiCommand,
    ) -> Option<UiCommand> {
        match command {
            UiCommand::Stats {
                request,
                dataset_id,
            } => self.spawn_message(request.clone(), async move {
                let result = api
                    .dataset_stats(&dataset_id)
                    .await
                    .map_err(|error| error.to_string());
                UiMessage::StatsLoaded { request, result }
            }),
            UiCommand::AssignmentAvailability {
                request,
                dataset_id,
                kind,
            } => self.spawn_message(request.clone(), async move {
                let result = api
                    .assignment_availability(
                        &dataset_id,
                        labello_client::AssignmentAvailabilityRequest { kind },
                    )
                    .await
                    .map_err(|error| error.to_string());
                UiMessage::AssignmentAvailabilityLoaded { request, result }
            }),
            UiCommand::SaveKeybindings {
                request,
                dataset_id,
                keybindings,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::KeybindingsSaved {
                    request,
                    result: api
                        .save_keybindings(&dataset_id, keybindings)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            command => return Some(command),
        }
        None
    }
}

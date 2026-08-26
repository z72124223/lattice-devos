use lattice_contracts::{ContentDigest, ProjectId, ProjectSnapshotId, TaskId, TaskIntakeBinding};
use lattice_orchestrator::{GeneralTaskIntakeRequest, create_general_task};
use lattice_ports::{
    TaskIntakeAdmission, TaskIntakeLifecycleEvidence, TaskIntakeLifecyclePort, TaskLifecycleResult,
};
use lattice_task_domain::TaskState;

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn binding() -> TaskIntakeBinding {
    TaskIntakeBinding::new(
        ProjectId::new("project-general-intake").expect("project"),
        ProjectSnapshotId::new("snapshot-general-intake").expect("snapshot"),
        TaskId::new("task-general-intake").expect("task"),
        "1",
        digest('a'),
    )
    .expect("binding")
}

fn request() -> GeneralTaskIntakeRequest {
    GeneralTaskIntakeRequest::new(binding(), "client-general-intake-1").expect("bounded request")
}

struct IntakeLifecycle {
    binding: TaskIntakeBinding,
    admitted: bool,
    calls: Vec<&'static str>,
}

impl IntakeLifecycle {
    fn new() -> Self {
        Self {
            binding: binding(),
            admitted: false,
            calls: Vec::new(),
        }
    }

    fn evidence(&self) -> TaskIntakeLifecycleEvidence {
        TaskIntakeLifecycleEvidence::new(self.binding.clone(), digest('d'))
            .expect("intake evidence")
    }
}

impl TaskIntakeLifecyclePort for IntakeLifecycle {
    fn admit(
        &mut self,
        binding: &TaskIntakeBinding,
        client_request_id: &str,
    ) -> TaskLifecycleResult<TaskIntakeAdmission> {
        assert_eq!(binding, &self.binding);
        assert_eq!(client_request_id, "client-general-intake-1");
        self.calls.push("admit");
        let evidence = self.evidence();
        if self.admitted {
            Ok(TaskIntakeAdmission::exact_replay(evidence))
        } else {
            self.admitted = true;
            Ok(TaskIntakeAdmission::created(evidence))
        }
    }

    fn load(
        &mut self,
        binding: &TaskIntakeBinding,
    ) -> TaskLifecycleResult<TaskIntakeLifecycleEvidence> {
        assert_eq!(binding, &self.binding);
        self.calls.push("load");
        Ok(self.evidence())
    }
}

#[test]
fn general_task_intake_is_one_admit_call_and_exactly_replays_without_autonomy() {
    let request = request();
    let mut lifecycle = IntakeLifecycle::new();

    let created = create_general_task(&request, &mut lifecycle).expect("create task");
    assert_eq!(created.binding(), request.binding());
    assert_eq!(created.state(), TaskState::Draft);
    assert!(created.result_digest().is_none());
    assert!(!created.is_exact_replay());
    assert_eq!(lifecycle.calls, ["admit"]);

    let replayed = create_general_task(&request, &mut lifecycle).expect("exact replay");
    assert_eq!(replayed.evidence(), created.evidence());
    assert!(replayed.is_exact_replay());
    assert_eq!(lifecycle.calls, ["admit", "admit"]);
}

struct SubstitutedBindingLifecycle;

impl TaskIntakeLifecyclePort for SubstitutedBindingLifecycle {
    fn admit(
        &mut self,
        _binding: &TaskIntakeBinding,
        _client_request_id: &str,
    ) -> TaskLifecycleResult<TaskIntakeAdmission> {
        let substituted = TaskIntakeBinding::new(
            ProjectId::new("project-substituted").expect("project"),
            ProjectSnapshotId::new("snapshot-substituted").expect("snapshot"),
            TaskId::new("task-substituted").expect("task"),
            "1",
            digest('b'),
        )
        .expect("substituted binding");
        let evidence = TaskIntakeLifecycleEvidence::new(substituted, digest('c'))?;
        Ok(TaskIntakeAdmission::created(evidence))
    }

    fn load(
        &mut self,
        _binding: &TaskIntakeBinding,
    ) -> TaskLifecycleResult<TaskIntakeLifecycleEvidence> {
        panic!("create-only coordinator must not perform a second load")
    }
}

#[test]
fn general_task_intake_rejects_adapter_binding_substitution() {
    let request = request();
    let error = create_general_task(&request, &mut SubstitutedBindingLifecycle)
        .expect_err("adapter must not substitute intake identity");
    assert_eq!(
        error.to_string(),
        "general task intake lifecycle state mismatch"
    );
}

#[test]
fn general_task_intake_rejects_unbounded_client_request_id() {
    assert!(GeneralTaskIntakeRequest::new(binding(), "a".repeat(64)).is_ok());
    for rejected in [
        " contains-spaces ".to_owned(),
        "a".repeat(65),
        "sk-do-not-use".to_owned(),
        "token:do-not-use".to_owned(),
    ] {
        let error =
            GeneralTaskIntakeRequest::new(binding(), rejected).expect_err("unsafe identifier");
        assert_eq!(error.to_string(), "general task intake request rejected");
    }
}

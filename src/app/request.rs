use crate::args::RequestArgs;
use crate::output::{RequestOutput, format_response, print_json};
use crate::page::PageSession;
use crate::queue::{Queue, QueueStatus, StatusResult};
use crate::scinet::{
    RequestRemoteState, SCINET_URL, ScinetResponse, probe_current_session, request_doi,
    view_request,
};

pub(super) fn handle_request(queue: &Queue, request: RequestArgs) -> Result<(), String> {
    let dois = request_dois(queue, &request)?;

    if dois.is_empty() {
        if request.json {
            print_json(&Vec::<RequestOutput>::new())?;
            return Ok(());
        }

        println!("no queued entries");
        return Ok(());
    }

    let results = super::with_scinet_page(!request.json, |page| {
        let mut results = Vec::new();

        if request.budget_check {
            let probe = probe_current_session(page).map_err(|error| error.to_string())?;
            ensure_budget(probe.token_balance, request.reward, dois.len())?;
        }

        for doi in &dois {
            let response = request_doi(page, SCINET_URL, doi, request.reward)
                .map_err(|error| error.to_string())?;

            if response.looks_logged_out() {
                return Err("not logged into Sci-Net; run `snq login` first".to_string());
            }

            let (status, remote_state) = record_request_or_existing(page, queue, doi, &response)?;
            results.push(RequestOutput {
                doi: doi.clone(),
                status,
                remote_state,
                response,
            });
        }

        Ok(results)
    })?;

    if request.json {
        print_json(&results)?;
        return Ok(());
    }

    for result in results {
        match result.remote_state {
            Some(remote_state) => println!("already-{}\t{}", remote_state.as_str(), result.doi),
            None if request.all => println!("requested\t{}", result.doi),
            None => println!("{}", format_response(&result.response)?),
        }
    }

    Ok(())
}

fn ensure_budget(balance: Option<u32>, reward: u32, count: usize) -> Result<(), String> {
    let Some(balance) = balance else {
        return Err(
            "request: could not determine Sci-Net token balance; retry without --budget-check to let Sci-Net decide"
                .to_string(),
        );
    };
    let count = u64::try_from(count).map_err(|_| "request: too many request targets")?;
    let required = u64::from(reward)
        .checked_mul(count)
        .ok_or("request: budget calculation overflowed")?;

    if u64::from(balance) < required {
        return Err(format!(
            "request: budget check failed: {required} tokens required, {balance} available"
        ));
    }

    Ok(())
}

fn request_dois(queue: &Queue, request: &RequestArgs) -> Result<Vec<String>, String> {
    if let Some(doi) = &request.doi {
        return Ok(vec![doi.clone()]);
    }

    let entries = queue.list().map_err(|error| error.to_string())?;

    Ok(entries
        .into_iter()
        .filter(|entry| entry.status == QueueStatus::Queued)
        .map(|entry| entry.doi)
        .collect())
}

fn mark_requested(queue: &Queue, doi: &str) -> Result<(), String> {
    match queue
        .set_status(doi, QueueStatus::Requested)
        .map_err(|error| error.to_string())?
    {
        StatusResult::Updated(_) => {}
        StatusResult::NotFound(_) => {
            let _ = queue.add(doi).map_err(|error| error.to_string())?;
            let _ = queue
                .set_status(doi, QueueStatus::Requested)
                .map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

fn ensure_request_ok(doi: &str, response: &ScinetResponse) -> Result<(), String> {
    if response.ok {
        if let Some(error) = response.logical_error() {
            Err(format!(
                "request: Sci-Net returned logical error for {doi}: {error}"
            ))
        } else {
            Ok(())
        }
    } else {
        Err(format!(
            "request: Sci-Net returned status {} for {doi}",
            response.status
        ))
    }
}

fn record_request_or_existing(
    page: &mut impl PageSession,
    queue: &Queue,
    doi: &str,
    response: &ScinetResponse,
) -> Result<(QueueStatus, Option<RequestRemoteState>), String> {
    match ensure_request_ok(doi, response) {
        Ok(()) => {
            mark_requested(queue, doi)?;
            Ok((QueueStatus::Requested, None))
        }
        Err(error) => {
            let view = view_request(page, SCINET_URL, doi).map_err(|error| error.to_string())?;
            let remote_state = view.remote_state_for_doi(doi);

            if remote_state == RequestRemoteState::LoggedOut {
                return Err("not logged into Sci-Net; run `snq login` first".to_string());
            }

            if remote_state == RequestRemoteState::NotFound {
                return Err(error);
            }

            let status = match remote_state {
                RequestRemoteState::Working => {
                    mark_requested(queue, doi)?;
                    queue
                        .set_status(doi, QueueStatus::Working)
                        .map_err(|error| error.to_string())?;
                    QueueStatus::Working
                }
                RequestRemoteState::Pending | RequestRemoteState::Pdf => {
                    mark_requested(queue, doi)?;
                    QueueStatus::Requested
                }
                RequestRemoteState::LoggedOut | RequestRemoteState::NotFound => unreachable!(),
            };

            Ok((status, Some(remote_state)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::PageError;
    use std::fs;

    #[test]
    fn request_all_targets_only_queued_entries() {
        let dir = std::env::temp_dir().join(format!("snq-request-test-{}", std::process::id()));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);

        queue.add("10.1000/snq-example").unwrap();
        queue.add("10.1000/snq-alt").unwrap();
        queue
            .set_status("10.1000/snq-alt", QueueStatus::Requested)
            .unwrap();

        let request = RequestArgs {
            doi: None,
            reward: 1,
            all: true,
            budget_check: false,
            json: false,
        };

        assert_eq!(
            request_dois(&queue, &request).unwrap(),
            vec!["10.1000/snq-example".to_string()]
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn successful_request_is_marked_before_later_failure() {
        let dir = std::env::temp_dir().join(format!(
            "snq-request-partial-failure-test-{}",
            std::process::id()
        ));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);

        queue.add("10.1000/snq-example").unwrap();
        queue.add("10.1000/snq-alt").unwrap();

        let ok = ScinetResponse {
            ok: true,
            status: 200,
            body: serde_json::json!({ "success": true }),
        };
        let logical_error = ScinetResponse {
            ok: true,
            status: 200,
            body: serde_json::json!({ "success": false, "message": "invalid request" }),
        };

        let mut ok_page = FakePageSession::new(Vec::new());
        let mut error_page = FakePageSession::new(vec![serde_json::json!({
            "title": "Sci-Net",
            "url": "https://sci-net.xyz/",
            "text": "tokens request library",
            "pdf_urls": []
        })]);

        record_request_or_existing(&mut ok_page, &queue, "10.1000/snq-example", &ok).unwrap();
        assert!(
            record_request_or_existing(&mut error_page, &queue, "10.1000/snq-alt", &logical_error)
                .is_err()
        );

        let entries = queue.list().unwrap();
        assert_eq!(entries[0].status, QueueStatus::Requested);
        assert_eq!(entries[1].status, QueueStatus::Queued);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_request_syncs_existing_pending_page() {
        let dir = std::env::temp_dir().join(format!(
            "snq-request-existing-pending-test-{}",
            std::process::id()
        ));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);
        let doi = "10.1000/snq-existing";

        queue.add(doi).unwrap();
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: serde_json::json!({ "error": true }),
        };
        let mut page = FakePageSession::new(vec![serde_json::json!({
            "title": "Sci-Net",
            "url": "https://sci-net.xyz/10.1000/snq-existing",
            "text": "doi 10.1000/snq-existing\nReward: 1 token",
            "pdf_urls": []
        })]);

        let (status, remote_state) =
            record_request_or_existing(&mut page, &queue, doi, &response).unwrap();

        assert_eq!(status, QueueStatus::Requested);
        assert_eq!(remote_state, Some(RequestRemoteState::Pending));
        assert_eq!(queue.list().unwrap()[0].status, QueueStatus::Requested);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_request_syncs_existing_pdf_page_as_requested() {
        let dir = std::env::temp_dir().join(format!(
            "snq-request-existing-pdf-test-{}",
            std::process::id()
        ));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);
        let doi = "10.1000/snq-existing-pdf";

        queue.add(doi).unwrap();
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: serde_json::json!({ "error": true }),
        };
        let mut page = FakePageSession::new(vec![serde_json::json!({
            "title": "Sci-Net",
            "url": "https://sci-net.xyz/10.1000/snq-existing-pdf",
            "text": "doi 10.1000/snq-existing-pdf\nuploaded",
            "pdf_urls": ["https://sci-net.xyz/storage/snq-existing-pdf.pdf"]
        })]);

        let (status, remote_state) =
            record_request_or_existing(&mut page, &queue, doi, &response).unwrap();

        assert_eq!(status, QueueStatus::Requested);
        assert_eq!(remote_state, Some(RequestRemoteState::Pdf));
        assert_eq!(queue.list().unwrap()[0].status, QueueStatus::Requested);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_request_syncs_existing_working_page() {
        let dir = std::env::temp_dir().join(format!(
            "snq-request-existing-working-test-{}",
            std::process::id()
        ));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);
        let doi = "10.1000/snq-existing-working";

        queue.add(doi).unwrap();
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: serde_json::json!({ "error": true }),
        };
        let mut page = FakePageSession::new(vec![serde_json::json!({
            "title": "Sci-Net",
            "url": "https://sci-net.xyz/10.1000/snq-existing-working",
            "text": "doi 10.1000/snq-existing-working\nA member is working on solving this request.",
            "pdf_urls": []
        })]);

        let (status, remote_state) =
            record_request_or_existing(&mut page, &queue, doi, &response).unwrap();

        assert_eq!(status, QueueStatus::Working);
        assert_eq!(remote_state, Some(RequestRemoteState::Working));
        assert_eq!(queue.list().unwrap()[0].status, QueueStatus::Working);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_request_does_not_sync_unrelated_page() {
        let dir = std::env::temp_dir().join(format!(
            "snq-request-unrelated-page-test-{}",
            std::process::id()
        ));
        let path = dir.join("queue.jsonl");
        let queue = Queue::new(path);
        let doi = "10.1000/snq-missing";

        queue.add(doi).unwrap();
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: serde_json::json!({ "error": true }),
        };
        let mut page = FakePageSession::new(vec![serde_json::json!({
            "title": "Sci-Net",
            "url": "https://sci-net.xyz/",
            "text": "tokens request library",
            "pdf_urls": []
        })]);

        let error = record_request_or_existing(&mut page, &queue, doi, &response).unwrap_err();

        assert!(error.contains("error`=true"));
        assert_eq!(queue.list().unwrap()[0].status, QueueStatus::Queued);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn request_non_ok_response_is_rejected() {
        let response = ScinetResponse {
            ok: false,
            status: 500,
            body: serde_json::json!({ "error": "boom" }),
        };

        assert!(ensure_request_ok("10.1000/snq-alt", &response).is_err());
    }

    #[test]
    fn request_logical_error_response_is_rejected() {
        let response = ScinetResponse {
            ok: true,
            status: 200,
            body: serde_json::json!({ "success": false, "message": "invalid request" }),
        };

        let error = ensure_request_ok("10.1000/snq-alt", &response).unwrap_err();
        assert!(error.contains("invalid request"));
    }

    #[test]
    fn budget_check_allows_sufficient_balance() {
        ensure_budget(Some(3), 1, 3).unwrap();
        ensure_budget(Some(3), 3, 1).unwrap();
    }

    #[test]
    fn budget_check_rejects_missing_or_insufficient_balance() {
        assert!(
            ensure_budget(None, 1, 1)
                .unwrap_err()
                .contains("could not determine")
        );
        assert!(
            ensure_budget(Some(2), 1, 3)
                .unwrap_err()
                .contains("3 tokens required, 2 available")
        );
    }

    struct FakePageSession {
        values: Vec<serde_json::Value>,
    }

    impl FakePageSession {
        fn new(values: Vec<serde_json::Value>) -> Self {
            Self { values }
        }
    }

    impl PageSession for FakePageSession {
        fn navigate(&mut self, _url: &str) -> Result<(), PageError> {
            Ok(())
        }

        fn evaluate_json(&mut self, _expression: &str) -> Result<serde_json::Value, PageError> {
            if self.values.is_empty() {
                return Err(PageError::UnexpectedResponse(serde_json::json!({
                    "error": "missing fake response"
                })));
            }

            Ok(self.values.remove(0))
        }

        fn close_browser(&mut self) -> Result<(), PageError> {
            Ok(())
        }
    }
}

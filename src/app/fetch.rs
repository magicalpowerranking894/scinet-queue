use std::thread;
use std::time::Duration;

use crate::args::FetchArgs;
use crate::output::{FetchOutput, FetchOutputStatus, print_json};
use crate::papers::{FetchResult, fetch_dois, fetch_one};
use crate::queue::Queue;
use crate::scinet::{RequestRemoteState, ScinetAvailability, ScinetAvailabilityLink};

pub(super) fn handle_fetch(queue: &Queue, fetch: FetchArgs) -> Result<(), String> {
    let dois = fetch_dois(queue, fetch.doi.as_deref())?;

    if dois.is_empty() {
        if fetch.json {
            print_json(&Vec::<FetchOutput>::new())?;
            return Ok(());
        }

        println!("queue empty");
        return Ok(());
    }

    let outputs = super::with_scinet_page(!fetch.json, |page| {
        fetch_until_complete(
            &dois,
            fetch.wait,
            |doi| fetch_one(queue, page, doi, &fetch.out_dir),
            |remaining| {
                if fetch.json {
                    eprintln!(
                        "waiting for {remaining} PDF(s); next poll in {}s",
                        fetch.poll_secs
                    );
                } else {
                    println!(
                        "waiting for {remaining} PDF(s); next poll in {}s",
                        fetch.poll_secs
                    );
                }
                thread::sleep(Duration::from_secs(fetch.poll_secs));
            },
        )
    })?;

    if fetch.json {
        print_json(&outputs)?;
    } else {
        for output in &outputs {
            println!("{}", fetch_text_line(output));
        }
    }

    Ok(())
}

fn fetch_until_complete<F, W>(
    dois: &[String],
    wait: bool,
    mut fetch: F,
    mut wait_for_next_poll: W,
) -> Result<Vec<FetchOutput>, String>
where
    F: FnMut(&str) -> Result<FetchResult, String>,
    W: FnMut(usize),
{
    let mut pending = dois.to_vec();
    let mut outputs = Vec::new();

    loop {
        let mut next_pending = Vec::new();

        for doi in pending {
            match fetch(&doi)? {
                FetchResult::Fetched(path) => outputs.push(FetchOutput {
                    doi,
                    status: FetchOutputStatus::Fetched,
                    remote_state: RequestRemoteState::Pdf,
                    availability: Vec::new(),
                    availability_links: Vec::new(),
                    path: Some(path.display().to_string()),
                }),
                FetchResult::NoPdf {
                    remote_state,
                    availability,
                    availability_links,
                } => {
                    if wait && availability.is_empty() {
                        next_pending.push(doi);
                    } else {
                        outputs.push(FetchOutput {
                            doi,
                            status: FetchOutputStatus::NoPdf,
                            remote_state,
                            availability,
                            availability_links,
                            path: None,
                        });
                    }
                }
            }
        }

        if !wait || next_pending.is_empty() {
            return Ok(outputs);
        }

        wait_for_next_poll(next_pending.len());
        pending = next_pending;
    }
}

fn fetch_text_line(output: &FetchOutput) -> String {
    match output.path.as_deref() {
        Some(path) => path.to_string(),
        None if output.availability.is_empty() => {
            format!("no-pdf\t{}\t{}", output.remote_state.as_str(), output.doi)
        }
        None => format!(
            "no-pdf\t{}\t{}\tavailability={}{}",
            output.remote_state.as_str(),
            output.doi,
            format_availability(&output.availability),
            format_availability_links_suffix(&output.availability_links)
        ),
    }
}

fn format_availability(availability: &[ScinetAvailability]) -> String {
    availability
        .iter()
        .map(|availability| availability.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn format_availability_links_suffix(links: &[ScinetAvailabilityLink]) -> String {
    if links.is_empty() {
        return String::new();
    }

    format!(
        "\tavailability_links={}",
        links
            .iter()
            .map(|link| format!("{}:{}", link.source.as_str(), link.url))
            .collect::<Vec<_>>()
            .join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_wait_keeps_polling_until_all_targets_are_fetched() {
        let dois = vec!["10.1000/one".to_string(), "10.1000/two".to_string()];
        let mut calls = Vec::new();
        let mut waits = Vec::new();

        let outputs = fetch_until_complete(
            &dois,
            true,
            |doi| {
                calls.push(doi.to_string());

                match (calls.len(), doi) {
                    (1, "10.1000/one") => Ok(FetchResult::Fetched("papers/one.pdf".into())),
                    (2, "10.1000/two") => Ok(FetchResult::NoPdf {
                        remote_state: RequestRemoteState::Working,
                        availability: Vec::new(),
                        availability_links: Vec::new(),
                    }),
                    (3, "10.1000/two") => Ok(FetchResult::Fetched("papers/two.pdf".into())),
                    _ => panic!("unexpected fetch call sequence: {calls:?}"),
                }
            },
            |remaining| waits.push(remaining),
        )
        .unwrap();

        assert_eq!(
            calls,
            vec![
                "10.1000/one".to_string(),
                "10.1000/two".to_string(),
                "10.1000/two".to_string()
            ]
        );
        assert_eq!(waits, vec![1]);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].doi, "10.1000/one");
        assert_eq!(outputs[0].path.as_deref(), Some("papers/one.pdf"));
        assert_eq!(outputs[1].doi, "10.1000/two");
        assert_eq!(outputs[1].path.as_deref(), Some("papers/two.pdf"));
    }

    #[test]
    fn fetch_wait_stops_polling_targets_with_scinet_availability() {
        let dois = vec!["10.1000/open".to_string(), "10.1000/pending".to_string()];
        let mut calls = Vec::new();
        let mut waits = Vec::new();

        let outputs = fetch_until_complete(
            &dois,
            true,
            |doi| {
                calls.push(doi.to_string());

                match (calls.len(), doi) {
                    (1, "10.1000/open") => Ok(FetchResult::NoPdf {
                        remote_state: RequestRemoteState::Pending,
                        availability: vec![ScinetAvailability::OpenAccess],
                        availability_links: Vec::new(),
                    }),
                    (2, "10.1000/pending") => Ok(FetchResult::NoPdf {
                        remote_state: RequestRemoteState::Pending,
                        availability: Vec::new(),
                        availability_links: Vec::new(),
                    }),
                    (3, "10.1000/pending") => Ok(FetchResult::Fetched("papers/pending.pdf".into())),
                    _ => panic!("unexpected fetch call sequence: {calls:?}"),
                }
            },
            |remaining| waits.push(remaining),
        )
        .unwrap();

        assert_eq!(
            calls,
            vec![
                "10.1000/open".to_string(),
                "10.1000/pending".to_string(),
                "10.1000/pending".to_string()
            ]
        );
        assert_eq!(waits, vec![1]);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].doi, "10.1000/open");
        assert_eq!(
            outputs[0].availability,
            vec![ScinetAvailability::OpenAccess]
        );
        assert!(outputs[0].path.is_none());
        assert_eq!(outputs[1].doi, "10.1000/pending");
        assert_eq!(outputs[1].path.as_deref(), Some("papers/pending.pdf"));
    }

    #[test]
    fn fetch_without_wait_reports_no_pdf_outputs() {
        let dois = vec!["10.1000/one".to_string()];
        let outputs = fetch_until_complete(
            &dois,
            false,
            |_| {
                Ok(FetchResult::NoPdf {
                    remote_state: RequestRemoteState::Pending,
                    availability: vec![ScinetAvailability::SciHub],
                    availability_links: vec![ScinetAvailabilityLink {
                        source: ScinetAvailability::SciHub,
                        url: "https://sci-hub.example/10.1000/one".to_string(),
                    }],
                })
            },
            |_| panic!("non-waiting fetch should not sleep"),
        )
        .unwrap();

        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].doi, "10.1000/one");
        assert!(matches!(outputs[0].status, FetchOutputStatus::NoPdf));
        assert_eq!(outputs[0].remote_state, RequestRemoteState::Pending);
        assert_eq!(outputs[0].availability, vec![ScinetAvailability::SciHub]);
        assert_eq!(
            outputs[0].availability_links,
            vec![ScinetAvailabilityLink {
                source: ScinetAvailability::SciHub,
                url: "https://sci-hub.example/10.1000/one".to_string(),
            }]
        );
        assert!(outputs[0].path.is_none());
    }

    #[test]
    fn fetch_text_output_keeps_mixed_batch_statuses() {
        let outputs = [
            FetchOutput {
                doi: "10.1000/one".to_string(),
                status: FetchOutputStatus::Fetched,
                remote_state: RequestRemoteState::Pdf,
                availability: Vec::new(),
                availability_links: Vec::new(),
                path: Some("papers/one.pdf".to_string()),
            },
            FetchOutput {
                doi: "10.1000/two".to_string(),
                status: FetchOutputStatus::NoPdf,
                remote_state: RequestRemoteState::Working,
                availability: vec![ScinetAvailability::OpenAccess, ScinetAvailability::SciHub],
                availability_links: vec![ScinetAvailabilityLink {
                    source: ScinetAvailability::SciHub,
                    url: "https://sci-hub.example/10.1000/two".to_string(),
                }],
                path: None,
            },
        ];
        let lines = outputs.iter().map(fetch_text_line).collect::<Vec<_>>();

        assert_eq!(
            lines,
            vec![
                "papers/one.pdf".to_string(),
                "no-pdf\tworking\t10.1000/two\tavailability=open-access,sci-hub\tavailability_links=sci-hub:https://sci-hub.example/10.1000/two".to_string()
            ]
        );
    }
}

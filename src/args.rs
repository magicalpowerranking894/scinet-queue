use std::path::PathBuf;

use crate::queue::normalize_doi;

pub(crate) struct LoginArgs {
    pub(crate) wait: bool,
}

pub(crate) struct BrowsersArgs {
    pub(crate) action: BrowsersAction,
    pub(crate) json: bool,
}

pub(crate) enum BrowsersAction {
    List,
    Pick,
    Set(PathBuf),
    Clear,
}

pub(crate) struct RequestArgs {
    pub(crate) doi: Option<String>,
    pub(crate) reward: u32,
    pub(crate) all: bool,
    pub(crate) budget_check: bool,
    pub(crate) json: bool,
}

pub(crate) struct FetchArgs {
    pub(crate) doi: Option<String>,
    pub(crate) out_dir: PathBuf,
    pub(crate) wait: bool,
    pub(crate) poll_secs: u64,
    pub(crate) json: bool,
}

pub(crate) struct ViewArgs {
    pub(crate) doi: String,
    pub(crate) json: bool,
}

pub(crate) struct UrlArgs {
    pub(crate) doi: String,
}

pub(crate) struct ApproveArgs {
    pub(crate) doi: String,
    pub(crate) force: bool,
    pub(crate) json: bool,
}

pub(crate) fn parse_json_flag(
    command: &str,
    args: impl Iterator<Item = String>,
) -> Result<bool, String> {
    let mut json = false;

    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            unknown => return Err(format!("{command}: unknown option `{unknown}`")),
        }
    }

    Ok(json)
}

pub(crate) fn parse_login(args: impl Iterator<Item = String>) -> Result<LoginArgs, String> {
    let mut wait = true;

    for arg in args {
        match arg.as_str() {
            "--no-wait" => wait = false,
            unknown => return Err(format!("login: unknown option `{unknown}`")),
        }
    }

    Ok(LoginArgs { wait })
}

pub(crate) fn parse_browsers(args: impl Iterator<Item = String>) -> Result<BrowsersArgs, String> {
    let mut action = BrowsersAction::List;
    let mut json = false;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--pick" => set_browser_action(&mut action, BrowsersAction::Pick)?,
            "--clear" => set_browser_action(&mut action, BrowsersAction::Clear)?,
            "--set" => {
                let Some(path) = args.next() else {
                    return Err("browsers: missing value for --set".to_string());
                };

                set_browser_action(&mut action, BrowsersAction::Set(path.into()))?;
            }
            unknown => return Err(format!("browsers: unknown option `{unknown}`")),
        }
    }

    if json && matches!(action, BrowsersAction::Pick) {
        return Err("browsers: --pick cannot be used with --json".to_string());
    }

    Ok(BrowsersArgs { action, json })
}

fn set_browser_action(current: &mut BrowsersAction, next: BrowsersAction) -> Result<(), String> {
    if matches!(current, BrowsersAction::List) {
        *current = next;
        Ok(())
    } else {
        Err("browsers: choose only one of --pick, --set, or --clear".to_string())
    }
}

pub(crate) fn parse_request(args: impl Iterator<Item = String>) -> Result<RequestArgs, String> {
    let mut doi = None;
    let mut reward = 1;
    let mut all = false;
    let mut budget_check = false;
    let mut json = false;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--all" => all = true,
            "--budget-check" => budget_check = true,
            "--json" => json = true,
            "--reward" | "-r" => {
                let Some(value) = args.next() else {
                    return Err("request: missing value for --reward".to_string());
                };

                reward = value
                    .parse()
                    .map_err(|_| format!("request: invalid reward `{value}`"))?;

                if reward == 0 {
                    return Err("request: reward must be greater than zero".to_string());
                }
            }
            value if value.starts_with('-') => {
                return Err(format!("request: unknown option `{value}`"));
            }
            value => {
                if doi.is_some() {
                    return Err(format!("request: unexpected argument `{value}`"));
                }

                doi = Some(normalize_doi(value).map_err(|error| error.to_string())?);
            }
        }
    }

    if all && doi.is_some() {
        return Err("request: use either --all or one DOI".to_string());
    }

    if !all && doi.is_none() {
        return Err("request: missing DOI".to_string());
    }

    Ok(RequestArgs {
        doi,
        reward,
        all,
        budget_check,
        json,
    })
}

pub(crate) fn parse_view(args: impl Iterator<Item = String>) -> Result<ViewArgs, String> {
    let mut doi = None;
    let mut json = false;

    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            value if value.starts_with('-') => {
                return Err(format!("view: unknown option `{value}`"));
            }
            value => {
                if doi.is_some() {
                    return Err(format!("view: unexpected argument `{value}`"));
                }

                doi = Some(normalize_doi(value).map_err(|error| error.to_string())?);
            }
        }
    }

    let Some(doi) = doi else {
        return Err("view: missing DOI".to_string());
    };

    Ok(ViewArgs { doi, json })
}

pub(crate) fn parse_url(args: impl Iterator<Item = String>) -> Result<UrlArgs, String> {
    let mut doi = None;

    for arg in args {
        match arg.as_str() {
            value if value.starts_with('-') => {
                return Err(format!("url: unknown option `{value}`"));
            }
            value => {
                if doi.is_some() {
                    return Err(format!("url: unexpected argument `{value}`"));
                }

                doi = Some(normalize_doi(value).map_err(|error| error.to_string())?);
            }
        }
    }

    let Some(doi) = doi else {
        return Err("url: missing DOI".to_string());
    };

    Ok(UrlArgs { doi })
}

pub(crate) fn parse_approve(args: impl Iterator<Item = String>) -> Result<ApproveArgs, String> {
    let mut doi = None;
    let mut force = false;
    let mut json = false;

    for arg in args {
        match arg.as_str() {
            "--force" => force = true,
            "--json" => json = true,
            value if value.starts_with('-') => {
                return Err(format!("approve: unknown option `{value}`"));
            }
            value => {
                if doi.is_some() {
                    return Err(format!("approve: unexpected argument `{value}`"));
                }

                doi = Some(normalize_doi(value).map_err(|error| error.to_string())?);
            }
        }
    }

    let Some(doi) = doi else {
        return Err("approve: missing DOI".to_string());
    };

    Ok(ApproveArgs { doi, force, json })
}

pub(crate) fn parse_fetch(args: impl Iterator<Item = String>) -> Result<FetchArgs, String> {
    let mut doi = None;
    let mut out_dir = PathBuf::from("papers");
    let mut wait = false;
    let mut poll_secs = 30;
    let mut json = false;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--wait" => wait = true,
            "--json" => json = true,
            "--poll" => {
                let Some(value) = args.next() else {
                    return Err("fetch: missing value for --poll".to_string());
                };

                poll_secs = value
                    .parse()
                    .map_err(|_| format!("fetch: invalid poll interval `{value}`"))?;

                if poll_secs == 0 {
                    return Err("fetch: poll interval must be greater than zero".to_string());
                }
            }
            "--out" | "-o" => {
                let Some(value) = args.next() else {
                    return Err("fetch: missing value for --out".to_string());
                };

                out_dir = value.into();
            }
            value if value.starts_with('-') => {
                return Err(format!("fetch: unknown option `{value}`"));
            }
            value => {
                if doi.is_some() {
                    return Err(format!("fetch: unexpected argument `{value}`"));
                }

                doi = Some(normalize_doi(value).map_err(|error| error.to_string())?);
            }
        }
    }

    Ok(FetchArgs {
        doi,
        out_dir,
        wait,
        poll_secs,
        json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_waits_by_default_and_accepts_no_wait() {
        assert!(parse_login(std::iter::empty()).unwrap().wait);
        assert!(
            !parse_login(["--no-wait"].into_iter().map(str::to_string))
                .unwrap()
                .wait
        );
        assert!(parse_login(["--bad"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn browsers_accepts_json_and_preference_actions() {
        assert!(matches!(
            parse_browsers(["--json"].into_iter().map(str::to_string))
                .unwrap()
                .action,
            BrowsersAction::List
        ));

        assert!(matches!(
            parse_browsers(["--pick"].into_iter().map(str::to_string))
                .unwrap()
                .action,
            BrowsersAction::Pick
        ));

        assert!(matches!(
            parse_browsers(["--clear"].into_iter().map(str::to_string))
                .unwrap()
                .action,
            BrowsersAction::Clear
        ));

        let args = parse_browsers(["--set", "browser"].into_iter().map(str::to_string)).unwrap();
        assert!(
            matches!(args.action, BrowsersAction::Set(path) if path == std::path::Path::new("browser"))
        );
    }

    #[test]
    fn browsers_rejects_ambiguous_or_non_scriptable_options() {
        assert!(parse_browsers(["--set"].into_iter().map(str::to_string)).is_err());
        assert!(parse_browsers(["--set", "a", "--clear"].into_iter().map(str::to_string)).is_err());
        assert!(parse_browsers(["--pick", "--json"].into_iter().map(str::to_string)).is_err());
        assert!(parse_browsers(["--bad"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn request_accepts_doi_and_defaults_reward_to_one() {
        let args = parse_request(["10.1000/snq-example"].into_iter().map(str::to_string)).unwrap();

        assert_eq!(args.doi.as_deref(), Some("10.1000/snq-example"));
        assert_eq!(args.reward, 1);
        assert!(!args.all);
        assert!(!args.budget_check);
    }

    #[test]
    fn request_accepts_all_and_reward_flags() {
        let args =
            parse_request(["--all", "--reward", "3"].into_iter().map(str::to_string)).unwrap();

        assert!(args.doi.is_none());
        assert_eq!(args.reward, 3);
        assert!(args.all);
        assert!(!args.budget_check);
        assert!(!args.json);

        assert_eq!(
            parse_request(
                ["10.1000/snq-example", "-r", "2"]
                    .into_iter()
                    .map(str::to_string)
            )
            .unwrap()
            .reward,
            2,
        );
    }

    #[test]
    fn request_accepts_json_flag() {
        let args = parse_request(
            ["--all", "--reward", "1", "--budget-check", "--json"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        assert!(args.all);
        assert!(args.budget_check);
        assert!(args.json);
    }

    #[test]
    fn request_rejects_missing_invalid_and_ambiguous_values() {
        assert!(parse_request(std::iter::empty()).is_err());
        assert!(parse_request(["--all", "--reward", "0"].into_iter().map(str::to_string)).is_err());
        assert!(parse_request(["--all", "--reward"].into_iter().map(str::to_string)).is_err());
        assert!(parse_request(["--foo"].into_iter().map(str::to_string)).is_err());
        assert!(
            parse_request(
                ["--all", "10.1000/snq-example"]
                    .into_iter()
                    .map(str::to_string)
            )
            .is_err()
        );
    }

    #[test]
    fn approve_requires_doi_and_accepts_force() {
        let args = parse_approve(
            ["10.1000/snq-example", "--force"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        assert_eq!(args.doi, "10.1000/snq-example");
        assert!(args.force);
        assert!(!args.json);
        assert!(
            parse_approve(
                ["10.1000/snq-example", "--json"]
                    .into_iter()
                    .map(str::to_string)
            )
            .unwrap()
            .json
        );
        assert!(parse_approve(std::iter::empty()).is_err());
        assert!(parse_approve(["--bad"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn out_dir_defaults_to_papers() {
        let args = parse_fetch(std::iter::empty()).unwrap();

        assert_eq!(args.out_dir, PathBuf::from("papers"));
        assert!(!args.wait);
        assert_eq!(args.poll_secs, 30);
        assert!(!args.json);
    }

    #[test]
    fn out_dir_accepts_long_and_short_flags() {
        assert_eq!(
            parse_fetch(["--out", "inbox"].into_iter().map(str::to_string))
                .unwrap()
                .out_dir,
            PathBuf::from("inbox")
        );
        assert_eq!(
            parse_fetch(["-o", "papers"].into_iter().map(str::to_string))
                .unwrap()
                .out_dir,
            PathBuf::from("papers")
        );
    }

    #[test]
    fn out_dir_rejects_missing_and_unknown_values() {
        assert!(parse_fetch(["--out"].into_iter().map(str::to_string)).is_err());
        assert!(parse_fetch(["--bad"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn fetch_accepts_wait_and_poll_interval() {
        let args = parse_fetch(["--wait", "--poll", "5"].into_iter().map(str::to_string)).unwrap();

        assert!(args.wait);
        assert_eq!(args.poll_secs, 5);
        assert!(
            parse_fetch(["--json"].into_iter().map(str::to_string))
                .unwrap()
                .json
        );
    }

    #[test]
    fn fetch_rejects_invalid_poll_interval() {
        assert!(parse_fetch(["--poll"].into_iter().map(str::to_string)).is_err());
        assert!(parse_fetch(["--poll", "0"].into_iter().map(str::to_string)).is_err());
        assert!(parse_fetch(["--poll", "soon"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn fetch_accepts_optional_doi() {
        let args = parse_fetch(
            ["10.1000/snq-example", "--out", "papers"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        assert_eq!(args.doi.as_deref(), Some("10.1000/snq-example"));
        assert_eq!(args.out_dir, PathBuf::from("papers"));
    }

    #[test]
    fn view_accepts_json_flag() {
        let args = parse_view(
            ["10.1000/snq-example", "--json"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        assert_eq!(args.doi, "10.1000/snq-example");
        assert!(args.json);
        assert!(parse_view(std::iter::empty()).is_err());
        assert!(parse_view(["--bad"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn url_requires_exactly_one_doi() {
        let args = parse_url(["10.1000/SNQ-EXAMPLE"].into_iter().map(str::to_string)).unwrap();

        assert_eq!(args.doi, "10.1000/snq-example");
        assert!(parse_url(std::iter::empty()).is_err());
        assert!(
            parse_url(
                ["10.1000/one", "10.1000/two"]
                    .into_iter()
                    .map(str::to_string)
            )
            .is_err()
        );
        assert!(parse_url(["--json"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn generic_json_flag_rejects_unknown_options() {
        assert!(parse_json_flag("list", ["--json"].into_iter().map(str::to_string)).unwrap());
        assert!(parse_json_flag("list", std::iter::empty()).is_ok());
        assert!(parse_json_flag("list", ["--bad"].into_iter().map(str::to_string)).is_err());
    }
}

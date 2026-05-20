use std::path::PathBuf;

use crate::queue::normalize_doi;

pub(crate) struct LoginArgs {
    pub(crate) wait: bool,
}

pub(crate) struct RequestArgs {
    pub(crate) doi: Option<String>,
    pub(crate) reward: u32,
    pub(crate) all: bool,
    pub(crate) json: bool,
}

pub(crate) struct FetchArgs {
    pub(crate) doi: Option<String>,
    pub(crate) out_dir: PathBuf,
    pub(crate) wait: bool,
    pub(crate) poll_secs: u64,
}

pub(crate) struct ViewArgs {
    pub(crate) doi: String,
    pub(crate) json: bool,
}

pub(crate) struct ApproveArgs {
    pub(crate) doi: String,
    pub(crate) force: bool,
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

pub(crate) fn parse_request(args: impl Iterator<Item = String>) -> Result<RequestArgs, String> {
    let mut doi = None;
    let mut reward = 1;
    let mut all = false;
    let mut json = false;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--all" => all = true,
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

pub(crate) fn parse_approve(args: impl Iterator<Item = String>) -> Result<ApproveArgs, String> {
    let mut doi = None;
    let mut force = false;

    for arg in args {
        match arg.as_str() {
            "--force" => force = true,
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

    Ok(ApproveArgs { doi, force })
}

pub(crate) fn parse_fetch(args: impl Iterator<Item = String>) -> Result<FetchArgs, String> {
    let mut doi = None;
    let mut out_dir = PathBuf::from("papers");
    let mut wait = false;
    let mut poll_secs = 30;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--wait" => wait = true,
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
    fn request_accepts_doi_and_defaults_reward_to_one() {
        let args =
            parse_request(["10.1287/mnsc.2024.05040"].into_iter().map(str::to_string)).unwrap();

        assert_eq!(args.doi.as_deref(), Some("10.1287/mnsc.2024.05040"));
        assert_eq!(args.reward, 1);
        assert!(!args.all);
    }

    #[test]
    fn request_accepts_all_and_reward_flags() {
        let args =
            parse_request(["--all", "--reward", "3"].into_iter().map(str::to_string)).unwrap();

        assert!(args.doi.is_none());
        assert_eq!(args.reward, 3);
        assert!(args.all);
        assert!(!args.json);

        assert_eq!(
            parse_request(
                ["10.1287/mnsc.2024.05040", "-r", "2"]
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
            ["--all", "--reward", "1", "--json"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        assert!(args.all);
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
                ["--all", "10.1287/mnsc.2024.05040"]
                    .into_iter()
                    .map(str::to_string)
            )
            .is_err()
        );
    }

    #[test]
    fn approve_requires_doi_and_accepts_force() {
        let args = parse_approve(
            ["10.1287/mnsc.2024.05040", "--force"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        assert_eq!(args.doi, "10.1287/mnsc.2024.05040");
        assert!(args.force);
        assert!(parse_approve(std::iter::empty()).is_err());
        assert!(parse_approve(["--bad"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn out_dir_defaults_to_papers() {
        let args = parse_fetch(std::iter::empty()).unwrap();

        assert_eq!(args.out_dir, PathBuf::from("papers"));
        assert!(!args.wait);
        assert_eq!(args.poll_secs, 30);
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
            ["10.1287/mnsc.2024.05040", "--out", "papers"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        assert_eq!(args.doi.as_deref(), Some("10.1287/mnsc.2024.05040"));
        assert_eq!(args.out_dir, PathBuf::from("papers"));
    }

    #[test]
    fn view_accepts_json_flag() {
        let args = parse_view(
            ["10.1287/mnsc.2024.05040", "--json"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        assert_eq!(args.doi, "10.1287/mnsc.2024.05040");
        assert!(args.json);
        assert!(parse_view(std::iter::empty()).is_err());
        assert!(parse_view(["--bad"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn generic_json_flag_rejects_unknown_options() {
        assert!(parse_json_flag("list", ["--json"].into_iter().map(str::to_string)).unwrap());
        assert!(parse_json_flag("list", std::iter::empty()).is_ok());
        assert!(parse_json_flag("list", ["--bad"].into_iter().map(str::to_string)).is_err());
    }
}

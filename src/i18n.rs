pub fn get_best<'a, 'b, I, L, T: ?Sized>(
    items: I,
    mut prefered_langs: Vec<&'b str>,
) -> Option<(&'b str, &'a T)>
where
    I: IntoIterator<Item = (L, &'a T)>,
    L: PartialEq<&'b str>,
{
    prefered_langs.push("");
    let max_rank = prefered_langs.len() - 1;
    items
        .into_iter()
        .fold(None, |prefered, item| match prefered {
            None => {
                let rank = prefered_langs
                    .iter()
                    .position(|l| item.0 == l)
                    .unwrap_or(max_rank);
                Some((item, rank))
            }
            Some((pref_item, pref_rank)) => {
                let rank = prefered_langs
                    .iter()
                    .position(|l| item.0 == l)
                    .unwrap_or(max_rank);
                if rank < pref_rank {
                    Some((item, rank))
                } else {
                    Some((pref_item, pref_rank))
                }
            }
        })
        .map(|((_, item), rank)| (prefered_langs[rank], item))
}

pub fn xmpp_err_to_string<'a>(
    err: &'a xmpp_parsers::stanza_error::StanzaError,
    prefered_langs: Vec<&'a str>,
) -> (&'a str, String) {
    get_best(&err.texts, prefered_langs)
        .map(|(a, b)| (a, b.clone()))
        .or_else(|| {
            Some((
                "",
                format!(
                    "{}: {}",
                    err.type_,
                    match err.defined_condition {
                        xmpp_parsers::stanza_error::DefinedCondition::BadRequest => "bad-request",
                        xmpp_parsers::stanza_error::DefinedCondition::Conflict => "conflict",
                        xmpp_parsers::stanza_error::DefinedCondition::FeatureNotImplemented =>
                            "feature-not-implemented",
                        xmpp_parsers::stanza_error::DefinedCondition::Forbidden => "forbidden",
                        xmpp_parsers::stanza_error::DefinedCondition::Gone => "gone",
                        xmpp_parsers::stanza_error::DefinedCondition::InternalServerError =>
                            "internal-server-error",
                        xmpp_parsers::stanza_error::DefinedCondition::ItemNotFound =>
                            "item-not-found",
                        xmpp_parsers::stanza_error::DefinedCondition::JidMalformed =>
                            "jid-malformed",
                        xmpp_parsers::stanza_error::DefinedCondition::NotAcceptable =>
                            "not-acceptable",
                        xmpp_parsers::stanza_error::DefinedCondition::NotAllowed => "not-allowed",
                        xmpp_parsers::stanza_error::DefinedCondition::NotAuthorized =>
                            "not-authorized",
                        xmpp_parsers::stanza_error::DefinedCondition::PolicyViolation =>
                            "policy-violation",
                        xmpp_parsers::stanza_error::DefinedCondition::RecipientUnavailable =>
                            "recipient-unavailable",
                        xmpp_parsers::stanza_error::DefinedCondition::Redirect => "redirect",
                        xmpp_parsers::stanza_error::DefinedCondition::RegistrationRequired =>
                            "registration-required",
                        xmpp_parsers::stanza_error::DefinedCondition::RemoteServerNotFound =>
                            "remote-server-not-found",
                        xmpp_parsers::stanza_error::DefinedCondition::RemoteServerTimeout =>
                            "remote-server-timeout",
                        xmpp_parsers::stanza_error::DefinedCondition::ResourceConstraint =>
                            "resource-constraint",
                        xmpp_parsers::stanza_error::DefinedCondition::ServiceUnavailable =>
                            "service-unavailable",
                        xmpp_parsers::stanza_error::DefinedCondition::SubscriptionRequired =>
                            "subscription-required",
                        xmpp_parsers::stanza_error::DefinedCondition::UndefinedCondition =>
                            "undefined-condition",
                        xmpp_parsers::stanza_error::DefinedCondition::UnexpectedRequest =>
                            "unexpected-request",
                    }
                ),
            ))
        })
        .unwrap()
}

#[cfg(test)]
mod tests_command_parser {
    use super::*;

    #[test]
    fn test_get_best_lang() {
        // Given
        let items = vec![("", "orig"), ("fr", "français"), ("en", "english")];

        // When
        let best = get_best(items, vec!["fr"]);

        // Then
        assert_eq!(best, Some(("fr", "français")));
    }

    #[test]
    fn test_get_best_lang_without_pref() {
        // Given
        let items = vec![("", "orig"), ("fr", "français"), ("en", "english")];

        // When
        let best = get_best(items, vec![]);

        // Then
        assert_eq!(best, Some(("", "orig")));
    }
}

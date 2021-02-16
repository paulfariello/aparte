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

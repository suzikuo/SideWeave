//! Host URL envelope. Import parsing and the confirmation dialog belong to the plugin.
pub fn ai_manager_payload(raw: &str) -> Option<String> {
    if raw.len()>128*1024 {return None}
    let url=url::Url::parse(raw).ok()?;
    if url.scheme()!="sideweave"||url.host_str()!=Some("plugin")||url.path()!="/ai_manager/import"||!url.username().is_empty()||url.password().is_some()||url.port().is_some()||url.fragment().is_some(){return None}
    let pairs=url.query_pairs().collect::<Vec<_>>();
    if pairs.len()!=1||pairs[0].0!="url"{return None}
    let inner=url::Url::parse(&pairs[0].1).ok()?;
    if inner.scheme()!="ccswitch"||inner.host_str()!=Some("v1")||inner.path()!="/import"{return None}
    Some(pairs[0].1.to_string())
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test] fn routes_only_the_import_envelope(){
        let inner="ccswitch://v1/import?resource=provider&app=codex&name=Fixture";
        let mut url=url::Url::parse("sideweave://plugin/ai_manager/import").unwrap();url.query_pairs_mut().append_pair("url",inner);
        assert_eq!(ai_manager_payload(url.as_str()).as_deref(),Some(inner));
        url.query_pairs_mut().append_pair("url",inner);assert!(ai_manager_payload(url.as_str()).is_none());
        for invalid in ["sideweave://plugin/other/import?url=ccswitch%3A%2F%2Fv1%2Fimport","sideweave://plugin/ai_manager/import?url=https%3A%2F%2Fexample.com","sideweave://install/plugin/ai_manager"]{assert!(ai_manager_payload(invalid).is_none())}
    }
}

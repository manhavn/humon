mod mongo;
mod query;
mod store;

/// macOS text substitutions turn typed `"` into `“`, which breaks query syntax.
/// WebKit reads these keys from the app's defaults; registering them (not
/// writing them) keeps it off without leaving anything on disk.
#[cfg(target_os = "macos")]
fn disable_text_substitutions() {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSNumber, NSString, NSUserDefaults};

    let keys = [
        "WebAutomaticQuoteSubstitutionEnabled",
        "WebAutomaticDashSubstitutionEnabled",
        "WebAutomaticTextReplacementEnabled",
        "WebAutomaticSpellingCorrectionEnabled",
    ]
    .map(NSString::from_str);
    let off = NSNumber::numberWithBool(false);
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let values: Vec<&AnyObject> = keys.iter().map(|_| off.as_ref()).collect();
    let dict = NSDictionary::from_slices(&key_refs, &values);
    // SAFETY: the dictionary maps NSString keys to NSNumber values, as registerDefaults expects.
    unsafe { NSUserDefaults::standardUserDefaults().registerDefaults(&dict) };
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "macos")]
    disable_text_substitutions();

    tauri::Builder::default()
        .manage(mongo::Connections::default())
        .invoke_handler(tauri::generate_handler![
            mongo::connect,
            mongo::connect_saved,
            mongo::disconnect,
            mongo::list_databases,
            mongo::list_collections,
            mongo::find_documents,
            mongo::count_documents,
            mongo::get_document,
            mongo::replace_document,
            mongo::delete_document,
            mongo::insert_documents,
            store::list_saved,
            store::save_connection,
            store::delete_saved,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

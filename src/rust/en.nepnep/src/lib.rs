#![no_std]
#![allow(clippy::mut_range_bound)]
use aidoku::{
	prelude::*, error::Result, std::String, std::Vec, std::net::Request, std::net::HttpMethod, std::html::Node,
	Filter, FilterType, Listing, Manga, MangaPageResult, Chapter, Page, DeepLink,
	std::defaults::defaults_get, std::json::parse,
};

pub mod helper;
mod parser;

static mut DIRECTORY_RID: Option<i32> = None;
static mut CACHED_MANGA_ID: Option<String> = None;
static mut CACHED_MANGA: Option<String> = None;
static mut COVER_SERVER: Option<String> = None;

// Cache full manga directory
// Done to avoid repeated requests and speed up parsing
pub fn initialize_directory() {
	if let Ok(url_str) = defaults_get("sourceURL").as_string() {
		let mut url = url_str.read();
		url.push_str("/search/");

		let html = Request::new(&url, HttpMethod::Get).html();
		let node = html.select("div.SearchResultCover img[ng-src]");
		unsafe {
			COVER_SERVER = Some(node.attr("ng-src").read());
		}

		let result = html.outer_html().read();
		let final_str = helper::string_between(&result, "vm.Directory = ", "];", 1);

		let mut directory_parsed = parse(final_str.as_bytes());
		directory_parsed.1 = false;
		unsafe {
			DIRECTORY_RID = Some(directory_parsed.0);
		}
	}
}

// Cache manga page html
pub fn cache_manga_page(id: &str) {
	if unsafe { CACHED_MANGA_ID.is_some() } && unsafe { CACHED_MANGA_ID.clone().unwrap() } == id {
		return;
	}
	if let Ok(url_str) = defaults_get("sourceURL").as_string() {
		let mut url = url_str.read();
		url.push_str("/manga/");
		url.push_str(id);
		unsafe { CACHED_MANGA = Some(Request::new(&url, HttpMethod::Get).string()) };
	}
}

#[get_manga_list]
fn get_manga_list(filters: Vec<Filter>, page: i32) -> Result<MangaPageResult> {
	if unsafe { DIRECTORY_RID.is_none() } {
		initialize_directory();
	}

	let mut manga: Vec<Manga> = Vec::new();

	let mut directory = unsafe { aidoku::std::ValueRef::new(DIRECTORY_RID.unwrap()) };
	directory.1 = false;

	let mut directory_arr = directory.as_array()?;

	let offset = (page as usize - 1) * 20;

	for filter in filters {
		match filter.kind {
			FilterType::Title => {
				let title = filter.value.as_string()?.read().to_lowercase();
				
				let mut i = 0;
				let mut size = directory_arr.len();
				for _ in 0..size {
					if i >= size || i >= offset + 20 {
						break;
					}
					let manga_title = match directory_arr.get(i).as_object()?.get("s").as_string() {
						Ok(title) => title.read().to_lowercase(),
						Err(_) => String::new(),
					};
					// check title
					if manga_title.contains(&title) {
						i += 1;
					} else {
						// check alt titles
						if let Ok(alt_titles) = directory_arr.get(i).as_object()?.get("al").as_array() {
							if alt_titles.into_iter().any(|a| {
								if let Ok(alt_title) = a.as_string() {
									alt_title.read().to_lowercase().contains(&title)
								} else {
									false
								}
							}) {
								i += 1;
								continue;
							}
						}
						// no match, remove
						directory_arr.remove(i);
						size -= 1;
					}
				}
			},
			FilterType::Sort => {
				// TODO
			},
			_ => continue,
		}
	}

	let end = if directory_arr.len() > offset + 20 {
		offset + 20
	} else {
		directory_arr.len()
	};

	for i in offset..end {
		let manga_obj = directory_arr.get(i).as_object()?;
		manga.push(parser::parse_basic_manga(manga_obj, unsafe { COVER_SERVER.clone().unwrap_or_default() })?);
	}

	Ok(MangaPageResult {
		manga,
		has_more: directory_arr.len() > end,
	})
}

#[get_manga_listing]
fn get_manga_listing(_listing: Listing, _page: i32) -> Result<MangaPageResult> {
	todo!()
}

#[get_manga_details]
fn get_manga_details(id: String) -> Result<Manga> {
	cache_manga_page(&id);
	unsafe { CACHED_MANGA_ID = Some(id.clone()) };
	let html = unsafe { Node::new(CACHED_MANGA.clone().unwrap().as_bytes()) };

	let mut url = defaults_get("sourceURL").as_string()?.read();
	url.push_str("/manga/");
	url.push_str(&id);

	parser::parse_full_manga(id, url, html)
}

#[get_chapter_list]
fn get_chapter_list(id: String) -> Result<Vec<Chapter>> {
	cache_manga_page(&id);
	unsafe { CACHED_MANGA_ID = Some(id.clone()) };
	let result = unsafe { CACHED_MANGA.clone().unwrap() };

	let start_loc = result.find("vm.Chapters = ").unwrap_or(0) + 14;
	let half_json = &result[start_loc..];
	let json_end = half_json.find("];").unwrap_or(half_json.len() - 1) + 1;
	let json = &half_json[..json_end];

	let chapter_arr = parse(json.as_bytes()).as_array()?;

	let mut chapters: Vec<Chapter> = Vec::new();

	for chapter in chapter_arr  {
		let chapter_obj = chapter.as_object()?;
		chapters.push(parser::parse_chapter(&id, chapter_obj)?);
	}

	Ok(chapters)
}

#[get_page_list]
fn get_page_list(id: String) -> Result<Vec<Page>> {
	let mut url = defaults_get("sourceURL").as_string()?.read();
	url.push_str("/read-online/");
	url.push_str(&id);

	let result = Request::new(&url, HttpMethod::Get).string();

	// create base image url
	let base_url = helper::string_between(&result, "vm.CurPathName = \"", "\";", 0);
	let title_uri = helper::string_between(&result, "vm.IndexName = \"", "\";", 0);

	let chapter = parse(helper::string_between(&result, "vm.CurChapter = ", "};", 1).as_bytes()).as_object()?;

	let directory = match chapter.get("Directory").as_string() {
		Ok(title) => title.read(),
		Err(_) => String::new(),
	};

	let mut base_path = String::from("https://");
	base_path.push_str(&base_url);
	base_path.push_str("/manga/");
	base_path.push_str(&title_uri);
	base_path.push('/');
	if !directory.is_empty() {
		base_path.push_str(&directory);
		base_path.push('/');
	}
	base_path.push_str(&helper::chapter_image(&chapter.get("Chapter").as_string()?.read(), true));

	let page_count = chapter.get("Page").as_int().unwrap_or(0);

	let mut pages: Vec<Page> = Vec::new();

	for i in 0..page_count {
		// pad page index to length 3 (e.g. 45 -> "046")
		let mut vec: Vec<u8> = Vec::new();
		let mut num = i as u8 + 1;
		loop {
			vec.insert(0, num % 10 + b'0');
			num /= 10;
			if num < 1 { break; }
		}
		while vec.len() < 3 {
			vec.insert(0, b'0');
		}

		let mut page_url = base_path.clone();
		page_url.push('-');
		page_url.push_str(&String::from_utf8(vec).unwrap_or_else(|_| String::from("000")));
		page_url.push_str(".png");

		pages.push(Page {
			index: i as i32,
			url: page_url,
			base64: String::new(),
			text: String::new(),
		})
	}

	Ok(pages)
}

#[handle_url]
fn hande_url(url: String) -> Result<DeepLink> {
	let mut url = &url[8..]; // remove "https://"
	let end = match url.find('/') {
		Some(i) => i + 1,
		None => url.len(),
	};
	url = &url[end..]; // remove url host

	if url.starts_with("manga/") {
		// ex: https://mangasee123.com/manga/Kanojo-Okarishimasu
		//     https://manga4life.com/manga/Kanojo-Okarishimasu

		let id = url.strip_prefix("manga/").unwrap_or_default(); // remove "manga/"
		let id_end = match id.find('/') {
			Some(i) => i,
			None => id.len(),
		};
		let manga_id = &id[..id_end];
		let manga = get_manga_details(String::from(manga_id))?;

		return Ok(DeepLink {
			manga: Some(manga),
			chapter: None,
		});
	} else if url.starts_with("read-online/") {
		// ex: https://manga4life.com/read-online/Kanojo-Okarishimasu-chapter-232.html

		let id = url.strip_prefix("read-online/").unwrap_or_default(); // remove "read-online/"
		let id_end = match id.find("-chapter") {
			Some(i) => i,
			None => id.len(),
		};
		let manga_id = &id[..id_end];
		let manga = get_manga_details(String::from(manga_id))?;

		return Ok(DeepLink {
			manga: Some(manga),
			chapter: None,
		});
	}

	Err(aidoku::error::AidokuError { reason: aidoku::error::AidokuErrorKind::Unimplemented })
}

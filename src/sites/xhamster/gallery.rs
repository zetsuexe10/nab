use serde_json::Value;

use super::json::{array, count, decode_entities, dig, first_text, text, tidy};
use crate::model::{Media, Paging, Photo, Video};
use crate::util;

#[derive(Clone, Debug)]
pub struct Gallery {
    pub item: Video,
    pub photos: Vec<Photo>,
    pub claimed: Option<u64>,
    pub paging: Paging,
}

fn id_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => None,
    }
}

pub fn id_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let segment = path.rsplit('/').find(|part| !part.is_empty())?;
    let digits: String = segment
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    (!digits.is_empty()).then_some(digits)
}

fn title_from_url(url: &str, id: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let segment = path.rsplit('/').find(|part| !part.is_empty()).unwrap_or("");
    let trimmed = segment
        .strip_suffix(&format!("-{id}"))
        .unwrap_or(segment)
        .replace('-', " ");
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        "gallery".to_string()
    } else {
        trimmed.to_string()
    }
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|raw| tidy(&decode_entities(&raw)))
        .filter(|found| !found.is_empty())
}

fn author(raw: Option<&Value>) -> (String, Option<String>) {
    let Some(node) = raw.filter(|node| node.is_object()) else {
        return (String::new(), None);
    };
    (
        clean(text(node.get("name"))).unwrap_or_default(),
        first_text(&[node.get("link"), node.get("pageURL")]),
    )
}

fn views_text(raw: &Value) -> String {
    if let Some(label) = clean(first_text(&[raw.get("viewsText"), raw.get("viewsLabel")])) {
        return label;
    }
    count(&[raw.get("views"), raw.get("viewsCount")])
        .map(|views| format!("{} views", util::short_count(views)))
        .unwrap_or_default()
}

fn photos_label(total: Option<u64>) -> String {
    match total {
        Some(1) => "1 photo".to_string(),
        Some(total) => format!("{total} photos"),
        None => String::new(),
    }
}

fn age(raw: &Value) -> String {
    count(&[raw.get("created")])
        .map(|created| util::ago(created, util::now()))
        .unwrap_or_default()
}

fn directory_item(raw: &Value) -> Option<Video> {
    let url = text(raw.get("link"))?;
    let id = id_text(raw.get("id")).or_else(|| id_from_url(&url))?;
    let (channel, author_url) = author(raw.get("author"));
    Some(Video {
        title: clean(text(raw.get("title"))).unwrap_or_else(|| title_from_url(&url, &id)),
        duration: photos_label(count(&[raw.get("photosCount")])),
        views: views_text(raw),
        published: age(raw),
        channel,
        author_url,
        url: Some(url),
        media: Media::Gallery,
        site: super::ID,
        id,
    })
}

fn person_item(raw: &Value) -> Option<Video> {
    let url = text(raw.get("pageURL"))?;
    let id = id_text(raw.get("id"))
        .or_else(|| id_text(raw.get("galleryID")))
        .or_else(|| id_from_url(&url))?;
    let (channel, author_url) = author(raw.get("author"));
    Some(Video {
        title: clean(first_text(&[raw.get("title"), raw.get("titleLocalized")]))
            .unwrap_or_else(|| title_from_url(&url, &id)),
        duration: photos_label(count(&[raw.get("quantity"), raw.get("imgCount")])),
        views: views_text(raw),
        published: age(raw),
        channel,
        author_url,
        url: Some(url),
        media: Media::Gallery,
        site: super::ID,
        id,
    })
}

fn shaped(items: &[Value], shape: fn(&Value) -> Option<Video>) -> Vec<Video> {
    let mut seen = std::collections::HashSet::new();
    items
        .iter()
        .filter_map(shape)
        .filter(|video| seen.insert(video.id.clone()))
        .collect()
}

pub fn listing(initials: &Value) -> Vec<Video> {
    let found = shaped(array(initials, "photosPage.galleriesItems"), directory_item);
    if !found.is_empty() {
        return found;
    }
    shaped(
        array(initials, "searchResultsPhotoComponent.list"),
        person_item,
    )
}

const PERSON_SHAPES: [(&str, &[&str]); 4] = [
    (
        "galleriesComponent.galleries",
        &["pagination", "galleriesComponent.pagination"],
    ),
    (
        "userGalleriesCollection",
        &["pagination", "galleriesComponent.pagination"],
    ),
    (
        "contentComponent.items",
        &["contentComponent.paginationProps"],
    ),
    (
        "layoutPage.galleriesListingProps.galleries",
        &["layoutPage.galleriesListingProps.pagination"],
    ),
];

pub fn person(initials: &Value) -> (Vec<Video>, Paging) {
    for (list, pagination) in PERSON_SHAPES {
        let found = shaped(array(initials, list), person_item);
        if !found.is_empty() {
            return (found, paging_at(initials, pagination));
        }
    }
    (Vec::new(), paging_at(initials, &["pagination"]))
}

pub fn person_total(initials: &Value) -> Option<u64> {
    count(&[dig(initials, "galleriesComponent.galleriesTotal")])
}

pub fn person_url(initials: &Value) -> Option<String> {
    text(dig(initials, "galleriesComponent.userPhotosUrl"))
}

pub fn directory_paging(initials: &Value) -> Paging {
    paging_at(
        initials,
        &[
            "photosPage.paginationProps",
            "searchResultsPhotoComponent.pagination",
            "pagination",
        ],
    )
}

pub fn paging_at(initials: &Value, paths: &[&str]) -> Paging {
    for path in paths {
        let Some(node) = dig(initials, path).filter(|node| node.is_object()) else {
            continue;
        };
        let page = count(&[
            node.get("currentPageNumber"),
            node.get("currentPage"),
            node.get("active"),
            node.get("page"),
        ]);
        let last = count(&[
            node.get("lastPageNumber"),
            node.get("maxPages"),
            node.get("maxPage"),
        ]);
        if page.is_some() || last.is_some() {
            let template = text(node.get("pageLinkTemplate")).filter(|link| link.contains("{#}"));
            return Paging {
                page: page.unwrap_or(1) as u32,
                last: last.unwrap_or(1).max(1) as u32,
                template,
            };
        }
    }
    Paging {
        page: 1,
        last: 1,
        template: None,
    }
}

fn photo_items(raw: &[Value]) -> Vec<Photo> {
    raw.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let url = first_text(&[row.get("imgSrc"), row.get("imgSrcset"), row.get("link")])?;
            Some(Photo {
                id: id_text(row.get("id")).unwrap_or_else(|| index.to_string()),
                url,
            })
        })
        .collect()
}

fn model_photos(raw: &[Value]) -> Vec<Photo> {
    raw.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let url = text(row.get("imageURL"))?;
            Some(Photo {
                id: id_text(row.get("id")).unwrap_or_else(|| index.to_string()),
                url,
            })
        })
        .collect()
}

pub fn photos(initials: &Value) -> Vec<Photo> {
    let found = photo_items(array(initials, "galleryPage.photoItems"));
    if !found.is_empty() {
        return found;
    }
    model_photos(array(initials, "galleryPage.galleryModel.photos"))
}

pub fn page(initials: &Value, url: &str) -> Gallery {
    let model = dig(initials, "galleryPage.galleryModel").unwrap_or(&Value::Null);
    let wrapper = dig(initials, "galleryPage").unwrap_or(&Value::Null);
    let found = photos(initials);
    let id = id_text(model.get("id"))
        .or_else(|| id_from_url(url))
        .unwrap_or_default();
    let claimed = count(&[
        model.get("quantity"),
        model.get("imgCount"),
        wrapper.get("photosCount"),
    ]);
    let writer = model
        .get("author")
        .filter(|node| node.is_object())
        .or_else(|| dig(initials, "galleryPage.author"));
    let (channel, author_url) = author(writer);
    let total = claimed.or((!found.is_empty()).then_some(found.len() as u64));
    Gallery {
        item: Video {
            title: clean(first_text(&[model.get("title"), wrapper.get("title")]))
                .unwrap_or_else(|| title_from_url(url, &id)),
            duration: photos_label(total),
            views: {
                let own = views_text(wrapper);
                if own.is_empty() {
                    views_text(model)
                } else {
                    own
                }
            },
            published: age(model),
            channel,
            author_url,
            url: Some(url.to_string()),
            media: Media::Gallery,
            site: super::ID,
            id,
        },
        photos: found,
        claimed,
        paging: paging_at(initials, &["galleryPage.paginationProps"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ids_and_titles_come_from_the_url() {
        assert_eq!(
            id_from_url("https://xhamster.com/photos/gallery/some-set-12345?x=1").as_deref(),
            Some("12345")
        );
        assert_eq!(
            id_from_url("https://xhamster.com/photos/gallery/none"),
            None
        );
        assert_eq!(
            title_from_url("https://x/p/some-set-12345", "12345"),
            "some set"
        );
    }

    #[test]
    fn directory_and_person_shapes_become_gallery_items() {
        let initials = json!({
            "photosPage": {"galleriesItems": [
                {"id": 7, "title": "A &amp; B", "link": "https://xhamster.com/photos/gallery/a-7",
                 "photosCount": 12, "views": 1500, "author": {"name": "Amy", "link": "https://xhamster.com/creators/amy"}},
                {"id": 7, "title": "dup", "link": "https://xhamster.com/photos/gallery/a-7"},
                {"title": "no link"}
            ]}
        });
        let items = listing(&initials);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "A & B");
        assert_eq!(items[0].duration, "12 photos");
        assert_eq!(items[0].views, "1.5K views");
        assert_eq!(items[0].channel, "Amy");
        assert_eq!(items[0].media, Media::Gallery);

        let initials = json!({
            "galleriesComponent": {"galleries": [
                {"galleryID": "9", "titleLocalized": "Nine", "pageURL": "https://xhamster.com/photos/gallery/nine-9", "quantity": 1}
            ], "galleriesTotal": 4, "userPhotosUrl": "https://xhamster.com/creators/amy/photos"},
            "pagination": {"currentPageNumber": 2, "lastPageNumber": 5, "pageLinkTemplate": "https://x/p/{#}"}
        });
        let (items, paging) = person(&initials);
        assert_eq!(items[0].id, "9");
        assert_eq!(items[0].duration, "1 photo");
        assert_eq!((paging.page, paging.last), (2, 5));
        assert_eq!(paging.template.as_deref(), Some("https://x/p/{#}"));
        assert_eq!(person_total(&initials), Some(4));
        assert_eq!(
            person_url(&initials).as_deref(),
            Some("https://xhamster.com/creators/amy/photos")
        );
    }

    #[test]
    fn gallery_pages_read_photos_from_either_shape() {
        let url = "https://xhamster.com/photos/gallery/set-55";
        let initials = json!({
            "galleryPage": {
                "photoItems": [
                    {"id": 1, "imgSrc": "https://i/1.jpg", "imgSrcset": "https://t/1.jpg"},
                    {"id": 2, "imgSrcset": "https://t/2.jpg"},
                    {"id": 3}
                ],
                "galleryModel": {"id": 55, "title": "Set", "quantity": 3, "author": {"name": "Zed", "pageURL": "https://xhamster.com/creators/zed"}},
                "paginationProps": {"currentPageNumber": 1, "lastPageNumber": 2, "pageLinkTemplate": "https://x/set-55/{#}"}
            }
        });
        let found = page(&initials, url);
        assert_eq!(found.item.id, "55");
        assert_eq!(found.item.title, "Set");
        assert_eq!(found.item.channel, "Zed");
        assert_eq!(found.photos.len(), 2);
        assert_eq!(found.photos[0].url, "https://i/1.jpg");
        assert_eq!(found.photos[1].url, "https://t/2.jpg");
        assert_eq!(found.claimed, Some(3));
        assert_eq!(found.paging.last, 2);

        let fallback = json!({"galleryPage": {"galleryModel": {"photos": [
            {"id": 4, "imageURL": "https://i/4.png"}, {"id": 5}
        ]}}});
        assert_eq!(
            photos(&fallback),
            vec![Photo {
                id: "4".into(),
                url: "https://i/4.png".into()
            }]
        );
    }
}

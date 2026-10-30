#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Media {
    #[default]
    Video,
    Gallery,
    Person,
}

#[derive(Clone, Debug, Default)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub duration: String,
    pub views: String,
    pub published: String,
    pub url: Option<String>,
    pub author_url: Option<String>,
    pub media: Media,
    pub site: &'static str,
}

impl Video {
    pub fn stub(id: &str) -> Self {
        Video {
            id: id.to_string(),
            ..Video::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Photo {
    pub id: String,
    pub url: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Scope {
    #[default]
    Videos,
    Galleries,
    Stars,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Videos => "videos",
            Scope::Galleries => "galleries",
            Scope::Stars => "stars",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paging {
    pub page: u32,
    pub last: u32,
    pub template: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub count: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    pub kind: &'static str,
    pub handle: Option<String>,
    pub verified: bool,
    pub country: Option<String>,
    pub gender: Option<String>,
    pub subscribers: Option<u64>,
    pub views: Option<u64>,
    pub video_count: Option<u64>,
    pub gallery_count: Option<u64>,
    pub aliases: Vec<String>,
    pub socials: Vec<(String, String)>,
    pub bio: Option<String>,
    pub info: Vec<(String, String)>,
    pub videos: Vec<Video>,
    pub scope: Scope,
    pub paging: Paging,
    pub url: String,
    pub sorts: Vec<Choice>,
    pub sort: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub text: String,
    pub likes: u64,
    pub created: Option<u64>,
    pub reply_to: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CommentPage {
    pub comments: Vec<Comment>,
    pub total: Option<u64>,
    pub page: u32,
    pub last: u32,
}

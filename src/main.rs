#![allow(warnings)]
use axum::{
    routing::{get, post},
    extract::{State, Form, Query, Path},
    response::{Html, IntoResponse, Redirect},
    Router,
};
use tower_cookies::{Cookie, CookieManagerLayer, Cookies};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, FromRow};
use bcrypt::{hash, DEFAULT_COST};
use std::net::SocketAddr;
use askama::Template;
use axum::http::StatusCode;
use axum_extra::extract::CookieJar;
use sqlx::Row;



#[derive(Clone)]
struct AppState {
    db: PgPool,
}

#[derive(serde::Deserialize)]
struct UpdateProfileRequest {
    new_password: Option<String>,
    avatar_url: Option<String>,
    birthday: Option<String>, // Додаємо поле для дати з форми
}
#[derive(Deserialize)]
struct FavoriteRequest {
    movie_id: i32,
    movie_title: String,
    poster_path: String,
}

#[derive(Deserialize)]
struct CommentForm {
    content: String,
    rating: i32,
}

#[derive(Deserialize, Serialize, Clone)]
struct Movie {
    id: i32,
    title: String,
    poster_path: Option<String>,
}

#[derive(Deserialize, Serialize, Clone)]
struct Genre {
    name: String,
}

#[derive(Deserialize, Serialize, Clone, Default)]
struct VideoResponse {
    results: Vec<Video>,
}

#[derive(Deserialize, Serialize, Clone)]
struct Video {
    key: String,
    site: String,
    #[serde(rename = "type")]
    video_type: String,
}

#[derive(Deserialize, Serialize, Clone)]
struct CastMember {
    name: String,
    character: String,
    profile_path: Option<String>,
}

#[derive(Deserialize, Serialize, Clone)]
struct CreditsResponse {
    cast: Vec<CastMember>,
}

#[derive(Deserialize, Serialize, Clone)]
struct RecommendationsResponse {
    results: Vec<Movie>,
}

#[derive(Deserialize, Serialize, Clone)]
struct MovieDetails {
    id: i32,
    title: String,
    overview: Option<String>,
    poster_path: Option<String>,
    release_date: Option<String>,
    vote_average: f32,
    genres: Vec<Genre>,
    #[serde(default)]
    videos: VideoResponse,
    #[serde(default)]
    credits: Option<CreditsResponse>,
    #[serde(default)]
    recommendations: Option<RecommendationsResponse>,
}

#[derive(Serialize, FromRow, Clone)]
struct Comment {
    id: i32,
    movie_id: i32,
    username: String,
    content: String,
    rating: Option<i32>,
    created_at: Option<chrono::NaiveDateTime>,
}

#[derive(Serialize, FromRow)]
struct FavoriteMovie {
    movie_id: i32,
    movie_title: String,
    poster_path: Option<String>,
}

#[derive(Deserialize)]
struct TmdbResponse {
    results: Vec<Movie>,
}

#[derive(Deserialize)]
struct GenreListResponse {
    genres: Vec<GenreItem>,
}

#[derive(Deserialize, Serialize, Clone)]
struct GenreItem {
    id: i32,
    name: String,
}

// Переконайся, що GenreItem та Movie мають #[derive(serde::Deserialize, Clone)]
#[derive(askama::Template)]
#[template(path = "index.html")] // Переконайся, що файл називається index.html
struct IndexTemplate {
    movies: Vec<Movie>,
    search_query: String,
    is_logged_in: bool,
    all_genres: Vec<GenreItem>,
    current_genre: i32,
    current_year: String,
    current_page: i32,    // ← додай
}


#[derive(Template)]
#[template(path = "auth.html")]
struct AuthTemplate;

#[derive(serde::Deserialize)]
struct RegisterRequest {
    username: String,
    password: String,
    avatar_url: Option<String>, // Додай це
    birthday: Option<String>,   // І це
}
#[derive(Template)]
#[template(path = "movie.html")]
struct MovieTemplate {
    movie: MovieDetails,
    is_logged_in: bool,
    comments: Vec<Comment>,
    is_admin: bool,  // ← додай
}


#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate {
    users: Vec<User>,
    total_users: i64,
    total_comments: i64,
    all_comments: Vec<Comment>,
}


#[derive(sqlx::FromRow, Deserialize, Serialize, Clone)]
pub struct User {
    pub id: i32,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub avatar_url: Option<String>,
    pub last_login: Option<chrono::NaiveDateTime>,
    // ПОЛЯ birthday, hours, status_phrase ТУТ НЕ ПОТРІБНІ,
    // бо вони рахуються окремо.
}

#[derive(Template)]
#[template(path = "profile.html")]
struct ProfileTemplate {
    username: String,
    avatar_url: String,
    birthday: Option<String>,
    favorites: Vec<FavoriteMovie>,
    watchlist: Vec<FavoriteMovie>,
    hours: i64,            // Ось тут вони живуть!
    status_phrase: String, // І тут!
}

// Додаємо параметри для пагінації та сортування
#[derive(Deserialize)]
struct SearchParams {
    query: Option<String>,
    genre: Option<i32>,
    year: Option<String>,
    page: Option<i32>,  // ← додай
}



// --- 3. ГОЛОВНА ФУНКЦІЯ ---

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPool::connect(&db_url).await.expect("DB connection failed");

    // Створення таблиць (Додаємо роль користувача для адмінки)
    let _ = sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id SERIAL PRIMARY KEY,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            role TEXT DEFAULT 'user'
        );"
    ).execute(&pool).await;
    let _ = sqlx::query(
        "CREATE TABLE IF NOT EXISTS watchlist (
        id SERIAL PRIMARY KEY,
        user_username TEXT NOT NULL,
        movie_id INTEGER NOT NULL,
        movie_title TEXT NOT NULL,
        poster_path TEXT
    );"
    ).execute(&pool).await;
    // Інші таблиці залишаються як були
    let _ = sqlx::query("CREATE TABLE IF NOT EXISTS favorites (id SERIAL PRIMARY KEY, user_username TEXT NOT NULL, movie_id INTEGER NOT NULL, movie_title TEXT NOT NULL, poster_path TEXT);").execute(&pool).await;
    let _ = sqlx::query("CREATE TABLE IF NOT EXISTS comments (id SERIAL PRIMARY KEY, movie_id INTEGER NOT NULL, username TEXT NOT NULL, content TEXT NOT NULL, rating INTEGER CHECK (rating >= 1 AND rating <= 10), created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP);").execute(&pool).await;

    let state = AppState { db: pool.clone() };

    let _ = sqlx::query(
        "ALTER TABLE comments ADD COLUMN IF NOT EXISTS username TEXT"
    ).execute(&pool).await;
    let _ = sqlx::query(
        "ALTER TABLE comments ADD COLUMN IF NOT EXISTS rating INTEGER CHECK (rating >= 1 AND rating <= 10)"
    ).execute(&pool).await;

    let cols = sqlx::query("SELECT column_name FROM information_schema.columns WHERE table_name = 'comments'")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
    for col in &cols {
        let name: String = col.get("column_name");
        println!("📋 Колонка: {}", name);
    }

    let app = Router::new()
        // --- ГОЛОВНА ТА ПОШУК ---
        .route("/", get(home_handler))

        // --- АВТОРИЗАЦІЯ (Вхід / Реєстрація / Вихід) ---
        .route("/auth", get(auth_page_handler))
        .route("/login", post(login_handler))
        .route("/register", post(register_handler))
        .route("/logout", get(logout_handler))

        // --- КОРИСТУВАЧ ТА ПРОФІЛЬ ---
        .route("/profile", get(profile_handler))
        .route("/profile/update", post(update_profile_handler)) // Оновлення пароля/аватарки

        // --- РОБОТА З ФІЛЬМАМИ ТА КОМЕНТАРЯМИ ---
        .route("/movie/:id", get(movie_details_handler))
        .route("/movie/:id/comment", post(add_comment_handler))

        // --- СПИСКИ (Обране та Дивитися пізніше) ---
        .route("/add_favorite", post(add_favorite_handler))
        .route("/delete_favorite/:movie_id", post(delete_favorite_handler))
        .route("/add_watchlist", post(add_watchlist_handler))      // Новий: Додати в Watchlist
        .route("/delete_watchlist/:movie_id", post(delete_watchlist_handler)) // Новий: Видалити з Watchlist
        .route("/watched/:id", post(add_to_watched_handler)) // Має бути POST


        // --- АДМІНІСТРУВАННЯ ---
        .route("/admin", get(admin_dashboard_handler))
        .route("/admin/delete_user/:id", post(delete_user_handler))
        .route("/admin/make_admin/:id", post(make_admin_handler))
        .route("/admin/delete_comment/:id", post(delete_comment_handler))

        // --- СЕРВІСНІ ШАРИ ---
        .layer(CookieManagerLayer::new())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("🚀 Сервер запущено на http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}


// --- 4. ОБРОБНИКИ (Handlers) ---

async fn register_handler(
    cookies: Cookies,
    State(state): State<AppState>,
    Form(payload): Form<RegisterRequest>
) -> impl IntoResponse {
    // 1. Хешуємо пароль
    let hashed = hash(&payload.password, DEFAULT_COST).expect("Error hashing password");

    // 2. Обробляємо дату народження (якщо вона є в структурі RegisterRequest)
    let birthday_parsed = payload.birthday.as_deref().and_then(|d| {
        chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()
    });

    // 3. Записуємо в базу всі дані, включаючи аватарку та день народження
    let result = sqlx::query(
        "INSERT INTO users (username, password_hash, avatar_url, birthday) VALUES ($1, $2, $3, $4)"
    )
        .bind(&payload.username)
        .bind(hashed)
        .bind(&payload.avatar_url)
        .bind(birthday_parsed) // Додаємо парсовану дату
        .execute(&state.db)
        .await;

    match result {
        Ok(_) => {
            println!("✅ Новий користувач: {}", payload.username);
            // ✅ АВТО-ВХІД: Ставимо куку і відправляємо в профіль
            cookies.add(Cookie::new("username", payload.username.clone()));
            Redirect::to("/profile").into_response()
        },
        Err(e) => {
            eprintln!("❌ Помилка реєстрації: {:?}", e);
            // ❌ Повертаємо на форму з параметром помилки для JS
            Redirect::to("/auth?error=exists").into_response()
        }
    }
}

async fn login_handler(
    cookies: Cookies,
    State(state): State<AppState>,
    Form(payload): Form<RegisterRequest>
) -> impl IntoResponse {
    // 1. Шукаємо юзера в базі за іменем
    let user_row: Option<(String, String)> = sqlx::query_as("SELECT username, password_hash FROM users WHERE username = $1")
        .bind(&payload.username)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);

    if let Some((username, stored_hash)) = user_row {
        // 2. Перевіряємо, чи підходить введений пароль до хешу в базі
        if bcrypt::verify(&payload.password, &stored_hash).unwrap_or(false) {

            // 3. Якщо пароль вірний — оновлюємо час входу
            let _ = sqlx::query("UPDATE users SET last_login = CURRENT_TIMESTAMP WHERE username = $1")
                .bind(&username)
                .execute(&state.db)
                .await;

            // 4. Додаємо куку та пускаємо в профіль
            cookies.add(Cookie::new("username", username));
            return Redirect::to("/profile").into_response();
        }
    }

    return Redirect::to("/auth?error=login").into_response();
}


async fn update_profile_handler(
    cookies: Cookies,
    State(state): State<AppState>,
    Form(payload): Form<UpdateProfileRequest>,
) -> impl IntoResponse {
    // 1. Авторизація
    let username = match cookies.get("username") {
        Some(c) => c.value().to_string(),
        None => return Redirect::to("/auth").into_response(),
    };

    // 2. Оновлення аватарки (через Option та фільтрацію порожніх рядків)
    if let Some(url) = payload.avatar_url.filter(|s| !s.trim().is_empty()) {
        let _ = sqlx::query("UPDATE users SET avatar_url = $1 WHERE username = $2")
            .bind(url)
            .bind(&username)
            .execute(&state.db)
            .await;
    }

    // 3. Оновлення дати народження (з парсингом у NaiveDate для PostgreSQL)
    if let Some(date_str) = payload.birthday.filter(|s| !s.trim().is_empty()) {
        if let Ok(parsed_date) = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
            let _ = sqlx::query("UPDATE users SET birthday = $1 WHERE username = $2")
                .bind(parsed_date)
                .bind(&username)
                .execute(&state.db)
                .await;
        }
    }

    // 4. Оновлення пароля
    if let Some(pass) = payload.new_password.filter(|s| !s.trim().is_empty()) {
        // Хешуємо пароль перед збереженням!
        if let Ok(hashed) = bcrypt::hash(&pass, bcrypt::DEFAULT_COST) {
            let _ = sqlx::query("UPDATE users SET password_hash = $1 WHERE username = $2")
                .bind(hashed)
                .bind(&username)
                .execute(&state.db)
                .await;
        }
    }

    // 5. [НОВЕ] Якщо ти захочеш додати зміну "Про мене" або статусів — додавай тут.

    Redirect::to("/profile").into_response()
}

async fn add_watchlist_handler(
    cookies: Cookies,
    State(state): State<AppState>,
    Form(p): Form<FavoriteRequest>,
) -> impl IntoResponse {
    // 1. Авторизація
    let username = match cookies.get("username") {
        Some(c) => c.value().to_string(),
        None => return Redirect::to("/auth").into_response(),
    };

    let api_key = "d8c185f7a099940bb875140c61700a26";
    let mut runtime = 120; // Стандарт, якщо API підведе

    // 2. Дізнаємося тривалість фільму ВІДРАЗУ при додаванні в список
    let url = format!(
        "https://api.themoviedb.org/3/movie/{}?api_key={}&language=uk-UA",
        p.movie_id, api_key
    );

    if let Ok(resp) = reqwest::get(url).await {
        if let Ok(details) = resp.json::<serde_json::Value>().await {
            if let Some(r) = details["runtime"].as_i64() {
                if r > 0 { runtime = r as i32; }
            }
        }
    }

    // 3. Записуємо в БД (Додай колонку runtime в таблицю watchlist, якщо хочеш її там зберігати)
    // Або просто залишаємо як є, а runtime будемо брати вже в add_to_watched_handler.

    let _ = sqlx::query(
        "INSERT INTO watchlist (user_username, movie_id, movie_title, poster_path)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_username, movie_id) DO NOTHING"
    )
        .bind(&username)
        .bind(p.movie_id)
        .bind(&p.movie_title)
        .bind(&p.poster_path)
        .execute(&state.db)
        .await;

    println!("📌 Фільм '{}' додано в план перегляду ({})", p.movie_title, username);

    Redirect::to("/profile").into_response()
}



async fn delete_watchlist_handler(
    cookies: Cookies,
    State(state): State<AppState>,
    Path(movie_id): Path<i32>,
) -> impl IntoResponse {
    if let Some(user_cookie) = cookies.get("username") {
        let _ = sqlx::query("DELETE FROM watchlist WHERE user_username = $1 AND movie_id = $2")
            .bind(user_cookie.value())
            .bind(movie_id)
            .execute(&state.db)
            .await;
    }
    Redirect::to("/profile").into_response()
}

async fn home_handler(
    cookies: Cookies,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let is_logged_in = cookies.get("username").is_some();
    let api_key = "d8c185f7a099940bb875140c61700a26";

    let genres_url = format!("https://api.themoviedb.org/3/genre/movie/list?api_key={}&language=uk-UA", api_key);
    let mut all_genres: Vec<GenreItem> = Vec::new();
    if let Ok(resp) = reqwest::get(genres_url).await {
        if let Ok(res) = resp.json::<GenreListResponse>().await {
            all_genres = res.genres;
        }
    }

    let current_page = params.page.unwrap_or(1).max(1);
    let year_param = params.year.as_deref().unwrap_or("").to_string();

    let url = match &params.query {
        Some(query) if !query.trim().is_empty() => {
            let mut base = format!(
                "https://api.themoviedb.org/3/search/movie?api_key={}&language=uk-UA&query={}&page={}",
                api_key, urlencoding::encode(query), current_page
            );
            if !year_param.is_empty() {
                base.push_str(&format!("&primary_release_year={}", year_param));
            }
            base
        },
        _ => {
            let mut base = format!(
                "https://api.themoviedb.org/3/discover/movie?api_key={}&language=uk-UA&page={}&sort_by=popularity.desc",
                api_key, current_page
            );
            if let Some(genre_id) = params.genre {
                if genre_id != -1 {
                    base.push_str(&format!("&with_genres={}", genre_id));
                }
            }
            if !year_param.is_empty() {
                base.push_str(&format!("&primary_release_year={}", year_param));
            }
            base
        }
    };

    let mut all_movies: Vec<Movie> = Vec::new();
    if let Ok(resp) = reqwest::get(url).await {
        if let Ok(res) = resp.json::<TmdbResponse>().await {
            all_movies = res.results;
        }
    }

    let template = IndexTemplate {
        movies: all_movies,
        search_query: params.query.clone().unwrap_or_default(),
        is_logged_in,
        all_genres,
        current_genre: params.genre.unwrap_or(-1),
        current_year: year_param,
        current_page,
    };

    // Знайти в кінці home_handler:
    match template.render() {
        Ok(html) => (
            [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
            Html(html)
        ).into_response(),
        Err(e) => {
            eprintln!("Template Error: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Помилка сервера").into_response()
        }
    }

}

async fn movie_details_handler(
    cookies: Cookies,
    Path(id): Path<i32>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let is_logged_in = cookies.get("username").is_some();
    let api_key = "d8c185f7a099940bb875140c61700a26";

    let url = format!(
        "https://api.themoviedb.org/3/movie/{}?api_key={}&language=uk-UA&append_to_response=videos,credits,recommendations",
        id, api_key
    );

    let response = match reqwest::get(&url).await {
        Ok(res) => res,
        Err(_) => return Redirect::to("/").into_response(),
    };

    let movie = match response.json::<MovieDetails>().await {
        Ok(m) => m,
        Err(_) => return Redirect::to("/").into_response(),
    };

    let comments = sqlx::query_as::<_, Comment>(
        "SELECT id, movie_id, username, content, rating, created_at FROM comments WHERE movie_id = $1 ORDER BY created_at DESC"
    )
        .bind(id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let is_admin = if let Some(user_cookie) = cookies.get("username") {
        sqlx::query_scalar::<_, String>("SELECT role FROM users WHERE username = $1")
            .bind(user_cookie.value())
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None)
            .map(|r| r == "admin")
            .unwrap_or(false)
    } else {
        false
    };

    // В кінці movie_details_handler:
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(MovieTemplate {
            movie,
            is_logged_in,
            comments,
            is_admin,
        }.render().unwrap())
    ).into_response()


}

async fn add_comment_handler(
    cookies: Cookies,
    State(state): State<AppState>,
    Path(movie_id): Path<i32>,
    Form(payload): Form<CommentForm>,
) -> impl IntoResponse {
    if let Some(user_cookie) = cookies.get("username") {
        let result = sqlx::query(
            "INSERT INTO comments (movie_id, user_username, content, rating, username) VALUES ($1, $2, $3, $4, $2)"
        )
            .bind(movie_id)
            .bind(user_cookie.value())
            .bind(&payload.content)
            .bind(payload.rating)
            .execute(&state.db)
            .await;

        match result {
            Ok(_) => println!("✅ Коментар додано від {}", user_cookie.value()),
            Err(e) => eprintln!("❌ Помилка додавання коментаря: {:?}", e),
        }
    } else {
        println!("❌ Користувач не залогінений");
    }

    Redirect::to(&format!("/movie/{}", movie_id)).into_response()
}


async fn auth_page_handler() -> impl IntoResponse { Html(AuthTemplate.render().unwrap()) }

async fn logout_handler(cookies: Cookies) -> impl IntoResponse {
    cookies.remove(Cookie::from("username"));
    Redirect::to("/")
}

async fn profile_handler(cookies: Cookies, State(state): State<AppState>) -> impl IntoResponse {
    // 1. Авторизація
    let username = match cookies.get("username") {
        Some(c) => c.value().to_string(),
        None => return Redirect::to("/auth").into_response(),
    };

    // 2. Отримуємо дані профілю (avatar та birthday)
    // Використовуємо звичайний query_as для простоти
    let user_data: (Option<String>, Option<chrono::NaiveDate>) =
        sqlx::query_as("SELECT avatar_url, birthday FROM users WHERE username = $1")
            .bind(&username)
            .fetch_one(&state.db)
            .await
            .unwrap_or((None, None));

    // 3. Рахуємо години (з таблиці watched)
    // Додаємо ::bigint у SQL, щоб точно отримати i64 для Rust
    let total_minutes: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(runtime), 0)::bigint FROM watched WHERE user_username = $1"
    )
        .bind(&username)
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);

    let hours = total_minutes / 60;

    // 4. Логіка статусів
    let status_phrase = match hours {
        0..=5 => "Кіно-немовля 👶",
        6..=20 => "Початківець 🍿",
        21..=100 => "Кіно-задрот 👓",
        _ => "Легенда CineRust 🐉",
    };

    // 5. Списки фільмів
    let favorites = sqlx::query_as::<_, FavoriteMovie>(
        "SELECT movie_id, movie_title, poster_path FROM favorites WHERE user_username = $1"
    )
        .bind(&username)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let watchlist = sqlx::query_as::<_, FavoriteMovie>(
        "SELECT movie_id, movie_title, poster_path FROM watchlist WHERE user_username = $1"
    )
        .bind(&username)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    // 6. Рендеринг
    let template = ProfileTemplate {
        username: username.clone(),
        avatar_url: user_data.0.unwrap_or_else(|| "https://cdn-icons-png.flaticon.com/512/149/149071.png".to_string()),
        birthday: user_data.1.map(|d| d.to_string()),
        favorites,
        watchlist,
        hours, // Передаємо як i64
        status_phrase: status_phrase.to_string(),
    };

    // В кінці profile_handler:
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(template.render().unwrap())
    ).into_response()

}


async fn add_favorite_handler(cookies: Cookies, State(state): State<AppState>, Form(p): Form<FavoriteRequest>) -> impl IntoResponse {
    if let Some(user_cookie) = cookies.get("username") {
        let _ = sqlx::query("INSERT INTO favorites (user_username, movie_id, movie_title, poster_path) VALUES ($1, $2, $3, $4)")
            .bind(user_cookie.value())
            .bind(p.movie_id)
            .bind(&p.movie_title)
            .bind(&p.poster_path)
            .execute(&state.db)
            .await;
    }
    Redirect::to("/profile")
}

async fn delete_favorite_handler(cookies: Cookies, State(state): State<AppState>, Path(movie_id): Path<i32>) -> impl IntoResponse {
    if let Some(user_cookie) = cookies.get("username") {
        let _ = sqlx::query("DELETE FROM favorites WHERE user_username = $1 AND movie_id = $2")
            .bind(user_cookie.value())
            .bind(movie_id)
            .execute(&state.db)
            .await;
    }
    Redirect::to("/profile")
}

async fn admin_dashboard_handler(
    cookies: Cookies,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let username = match cookies.get("username") {
        Some(c) => c.value().to_string(),
        None => return Redirect::to("/auth").into_response(),
    };

    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE username = $1")
        .bind(&username)
        .fetch_one(&state.db)
        .await
        .unwrap_or_else(|_| "user".to_string());

    if role.trim() != "admin" {
        return Redirect::to("/").into_response();
    }

    let total_users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db).await.unwrap_or(0);

    let total_comments: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM comments")
        .fetch_one(&state.db).await.unwrap_or(0);

    let users = sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, role, avatar_url, last_login FROM users ORDER BY id"
    )
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let all_comments = sqlx::query_as::<_, Comment>(
        "SELECT id, movie_id, username, content, rating, created_at FROM comments ORDER BY created_at DESC LIMIT 20"
    )
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    Html(AdminTemplate {
        users,
        total_users,
        total_comments,
        all_comments,
    }.render().unwrap()).into_response()
}

// Видалення користувача
async fn delete_user_handler(
    Path(id): Path<i32>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let _ = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    Redirect::to("/admin")
}

// НОВИЙ: Зміна ролі на адміна
async fn make_admin_handler(
    Path(id): Path<i32>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let _ = sqlx::query("UPDATE users SET role = 'admin' WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    Redirect::to("/admin")
}
async fn add_to_watched_handler(
    cookies: Cookies,
    State(state): State<AppState>,
    Path(movie_id): Path<i32>,
) -> impl IntoResponse {
    // 1. Перевіряємо, чи юзер залогінений
    let username = match cookies.get("username") {
        Some(c) => c.value().to_string(),
        None => return Redirect::to("/auth").into_response(),
    };

    // 2. Додаємо запис у таблицю статистики (120 хвилин за замовчуванням)
    let _ = sqlx::query(
        "INSERT INTO watched (user_username, movie_id, runtime) VALUES ($1, $2, $3)"
    )
        .bind(&username)
        .bind(movie_id)
        .bind(120) // Нараховуємо 2 години
        .execute(&state.db)
        .await;

    // 3. Видаляємо фільм із Watchlist
    let _ = sqlx::query(
        "DELETE FROM watchlist WHERE user_username = $1 AND movie_id = $2"
    )
        .bind(&username)
        .bind(movie_id)
        .execute(&state.db)
        .await;

    // 4. ВАЖЛИВО: Редирект назад у профіль, щоб не було білого екрана
    Redirect::to("/profile").into_response()
}
async fn delete_comment_handler(
    State(state): State<AppState>,
    Path(comment_id): Path<i32>,
    cookies: Cookies, // Використовуємо Cookies замість CookieJar
) -> impl IntoResponse {
    // 1. Отримуємо ім'я користувача з куки (як у твоїх інших функціях)
    let username = match cookies.get("username") {
        Some(c) => c.value().to_string(),
        None => return Redirect::to("/auth").into_response(),
    };

    // 2. Перевіряємо роль користувача в базі даних
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE username = $1")
        .bind(&username)
        .fetch_one(&state.db)
        .await
        .unwrap_or_else(|_| "user".to_string());

    if role.trim() != "admin" {
        println!("🚫 Спроба видалення коментаря без прав адміна: {}", username);
        return Redirect::to("/").into_response();
    }

    // 3. Дізнаємося movie_id перед видаленням, щоб знати, куди повернути адміна
    let movie_id_res = sqlx::query_scalar::<_, i32>(
        "SELECT movie_id FROM comments WHERE id = $1"
    )
        .bind(comment_id)
        .fetch_optional(&state.db)
        .await;

    // 4. Видаляємо коментар
    let _ = sqlx::query("DELETE FROM comments WHERE id = $1")
        .bind(comment_id)
        .execute(&state.db)
        .await;

    // 5. Повертаємо адміна назад на сторінку фільму
    match movie_id_res {
        Ok(Some(id)) => Redirect::to(&format!("/movie/{}", id)).into_response(),
        _ => Redirect::to("/admin").into_response(), // Якщо фільм не знайдено, в адмінку
    }
}

use assert_cmd::Command;
use mockito::Server;
use std::fs;
use tempfile::tempdir;

#[tokio::test]
async fn test_standard_crawl_and_assets() {
    let mut server = Server::new_async().await;
    
    // Mock the index page
    let _m_index = server.mock("GET", "/")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body(r#"
            <!DOCTYPE html>
            <html>
            <head>
                <title>Home</title>
                <link rel="stylesheet" href="/style.css">
            </head>
            <body>
                <h1>Home</h1>
                <a href="/about.html">About</a>
                <img src="/image.png">
            </body>
            </html>
        "#)
        .create_async().await;

    // Mock the about page
    let _m_about = server.mock("GET", "/about.html")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body(r#"
            <!DOCTYPE html>
            <html>
            <head>
                <title>About</title>
            </head>
            <body>
                <h1>About</h1>
            </body>
            </html>
        "#)
        .create_async().await;

    // Mock the CSS asset
    let _m_css = server.mock("GET", "/style.css")
        .with_status(200)
        .with_header("content-type", "text/css")
        .with_body("body { background: red; }")
        .create_async().await;

    // Mock the image asset
    let _m_img = server.mock("GET", "/image.png")
        .with_status(200)
        .with_header("content-type", "image/png")
        .with_body(vec![137, 80, 78, 71, 13, 10, 26, 10]) // Fake PNG bytes
        .create_async().await;

    // Create a temporary directory for output
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("backup");

    // Run the CLI
    let mut cmd = Command::cargo_bin("frontlane-static").unwrap();
    cmd.arg(server.url())
       .arg("--output")
       .arg(output_path.to_str().unwrap())
       .arg("--extract-metadata");

    cmd.assert().success();

    // Verify files were created
    assert!(output_path.join("index.html").exists(), "index.html was not created");
    assert!(output_path.join("about.html").exists(), "about.html was not created");
    assert!(output_path.join("style.css").exists(), "style.css was not created");
    assert!(output_path.join("image.png").exists(), "image.png was not created");
    assert!(output_path.join("metadata.csv").exists(), "metadata.csv was not created");

    // Verify metadata content
    let metadata_content = fs::read_to_string(output_path.join("metadata.csv")).unwrap();
    assert!(metadata_content.contains("Home"), "Metadata should contain the title of the home page");
    assert!(metadata_content.contains("About"), "Metadata should contain the title of the about page");
}

#[tokio::test]
async fn test_error_handling() {
    let mut server = Server::new_async().await;
    
    // Mock the index page linking to a 404
    let _m_index = server.mock("GET", "/")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body(r#"
            <!DOCTYPE html>
            <html>
            <body>
                <a href="/missing.html">Missing Page</a>
                <a href="/error.html">500 Page</a>
            </body>
            </html>
        "#)
        .create_async().await;

    let _m_404 = server.mock("GET", "/missing.html")
        .with_status(404)
        .with_body("Not Found")
        .create_async().await;
        
    let _m_500 = server.mock("GET", "/error.html")
        .with_status(500)
        .with_body("Internal Server Error")
        .create_async().await;

    // Create a temporary directory for output
    let temp_dir = tempdir().unwrap();
    let output_path = temp_dir.path().join("backup");

    // Run the CLI WITHOUT download-error-pages
    let mut cmd = Command::cargo_bin("frontlane-static").unwrap();
    cmd.arg(server.url())
       .arg("--output")
       .arg(output_path.to_str().unwrap());

    cmd.assert().success();

    // Verify index exists, but errors were skipped
    assert!(output_path.join("index.html").exists());
    assert!(!output_path.join("missing.html").exists());
    assert!(!output_path.join("error.html").exists());

    // Run again WITH download-error-pages
    let temp_dir_errors = tempdir().unwrap();
    let output_path_errors = temp_dir_errors.path().join("backup_errors");

    let mut cmd2 = Command::cargo_bin("frontlane-static").unwrap();
    cmd2.arg(server.url())
        .arg("--output")
        .arg(output_path_errors.to_str().unwrap())
        .arg("--download-error-pages");

    cmd2.assert().success();

    // Verify error pages were downloaded
    assert!(output_path_errors.join("index.html").exists());
    assert!(output_path_errors.join("missing.html").exists());
    assert!(output_path_errors.join("error.html").exists());
}

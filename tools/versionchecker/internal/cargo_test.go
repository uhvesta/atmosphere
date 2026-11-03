package versionchecker

import (
	"context"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(request *http.Request) (*http.Response, error) {
	return f(request)
}

func TestRustFixUsesParsedCargoManifest(t *testing.T) {
	root := t.TempDir()
	path := filepath.Join(root, "Cargo.toml")
	before := `[workspace]
members = []
# rust-version = "1.0.0"
[workspace.package]
edition = "2024"
rust-version = '1.98.1' # keep this comment
`
	if err := os.WriteFile(path, []byte(before), 0o644); err != nil {
		t.Fatal(err)
	}
	rust := Rust{Root: root}
	if current, err := rust.pin(); err != nil || current != "1.98.1" {
		t.Fatalf("pin = %q, %v", current, err)
	}
	if err := rust.Fix(context.Background(), Drift{Name: "rust-version", Current: "1.98.1", Latest: "1.99.0"}); err != nil {
		t.Fatal(err)
	}
	after, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	want := strings.Replace(before, "rust-version = '1.98.1'", `rust-version = "1.99.0"`, 1)
	if string(after) != want {
		t.Fatalf("Cargo.toml changed unexpectedly:\n%s", after)
	}
}

func TestCratesIgnoresBuildMetadata(t *testing.T) {
	root := t.TempDir()
	manifest := "[workspace.dependencies]\ntoml = \"1.1.8\"\n"
	if err := os.WriteFile(filepath.Join(root, "Cargo.toml"), []byte(manifest), 0o644); err != nil {
		t.Fatal(err)
	}
	crates := Crates{Root: root, Client: &http.Client{Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
		return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(strings.NewReader(`{"crate":{"max_stable_version":"1.1.8+spec-1.1.0"}}`)), Header: make(http.Header)}, nil
	})}}
	drifts, err := crates.Check(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(drifts) != 0 {
		t.Fatalf("got drifts for a build-metadata-only difference: %+v", drifts)
	}
}

func TestCratesChecksAndFixesWorkspaceDependencies(t *testing.T) {
	root := t.TempDir()
	path := filepath.Join(root, "Cargo.toml")
	before := `[workspace]
members = []

[workspace.dependencies]
serde = { features = ["derive"], version = "1.0.0" } # keep
renamed = { package = "real-crate", version = '0.4.0' }
local = { path = "../local" }
tokio = "1.0.0"

[workspace.dependencies.table_crate]
version = "2.0.0"
`
	if err := os.WriteFile(path, []byte(before), 0o644); err != nil {
		t.Fatal(err)
	}
	crates := Crates{Root: root, Client: &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
		if strings.HasSuffix(request.URL.Path, "/local") {
			t.Fatal("path dependency was queried")
		}
		if strings.HasSuffix(request.URL.Path, "/renamed") {
			t.Fatal("renamed alias was queried instead of package")
		}
		return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(strings.NewReader(`{"crate":{"max_stable_version":"9.0.0"}}`)), Header: make(http.Header)}, nil
	})}}
	drifts, err := crates.Check(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(drifts) != 4 {
		t.Fatalf("got %d crate drifts, want 4: %+v", len(drifts), drifts)
	}
	for _, drift := range drifts {
		if err := crates.Fix(context.Background(), drift); err != nil {
			t.Fatalf("fix %s: %v", drift.Name, err)
		}
	}
	if err := crates.FinishFix(context.Background()); err != nil {
		t.Fatal(err)
	}
	after, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	for _, old := range []string{`version = "1.0.0"`, `version = '0.4.0'`, `tokio = "1.0.0"`, `version = "2.0.0"`} {
		if strings.Contains(string(after), old) {
			t.Fatalf("old crate pin remains: %s", old)
		}
	}
	if !strings.Contains(string(after), `# keep`) || !strings.Contains(string(after), `local = { path = "../local" }`) {
		t.Fatalf("unrelated Cargo.toml content changed:\n%s", after)
	}
}

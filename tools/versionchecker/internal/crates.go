package versionchecker

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"sort"
	"strings"
)

type cratePin struct {
	name        string
	packageName string
	version     string
	inline      bool
}

// Crates checks registry dependencies pinned in [workspace.dependencies].
type Crates struct {
	Root        string
	CargoBinary string
	Client      *http.Client
}

func (Crates) Name() string { return "crates" }

func (c Crates) pins() ([]cratePin, error) {
	_, _, manifest, err := readCargo(c.Root)
	if err != nil {
		return nil, err
	}
	var pins []cratePin
	for name, value := range manifest.Workspace.Dependencies {
		pin := cratePin{name: name, packageName: name}
		switch dependency := value.(type) {
		case string:
			pin.version = dependency
		case map[string]any:
			if dependency["path"] != nil || dependency["git"] != nil || dependency["registry"] != nil || dependency["registry-index"] != nil {
				continue
			}
			pin.version, _ = dependency["version"].(string)
			if packageName, ok := dependency["package"].(string); ok {
				pin.packageName = packageName
			}
			pin.inline = true
		default:
			return nil, fmt.Errorf("workspace dependency %s has unsupported value", name)
		}
		if pin.version != "" {
			pins = append(pins, pin)
		}
	}
	sort.Slice(pins, func(i, j int) bool { return pins[i].name < pins[j].name })
	return pins, nil
}

func (c Crates) Check(ctx context.Context) ([]Drift, error) {
	pins, err := c.pins()
	if err != nil {
		return nil, err
	}
	client := c.Client
	if client == nil {
		client = http.DefaultClient
	}
	var drifts []Drift
	for _, pin := range pins {
		request, err := http.NewRequestWithContext(ctx, http.MethodGet, "https://crates.io/api/v1/crates/"+url.PathEscape(pin.packageName), nil)
		if err != nil {
			return nil, err
		}
		request.Header.Set("User-Agent", "monty-oxide-versionchecker")
		response, err := client.Do(request)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", pin.name, err)
		}
		if response.StatusCode != http.StatusOK {
			response.Body.Close()
			return nil, fmt.Errorf("%s: crates.io returned %s", pin.name, response.Status)
		}
		var result struct {
			Crate struct {
				MaxStableVersion string `json:"max_stable_version"`
			} `json:"crate"`
		}
		err = json.NewDecoder(response.Body).Decode(&result)
		response.Body.Close()
		if err != nil {
			return nil, fmt.Errorf("%s: %w", pin.name, err)
		}
		if result.Crate.MaxStableVersion == "" {
			return nil, fmt.Errorf("%s: no stable crate release", pin.name)
		}
		// Build metadata (toml's "1.1.8+spec-1.1.0") doesn't take part in
		// semver comparison, and Cargo ignores it in requirements.
		latest, _, _ := strings.Cut(result.Crate.MaxStableVersion, "+")
		if pin.version != latest {
			drifts = append(drifts, Drift{Name: pin.name, Current: pin.version, Latest: latest})
		}
	}
	return drifts, nil
}

func (c Crates) Fix(_ context.Context, drift Drift) error {
	pins, err := c.pins()
	if err != nil {
		return err
	}
	var selected *cratePin
	for i := range pins {
		if pins[i].name == drift.Name {
			selected = &pins[i]
			break
		}
	}
	if selected == nil || selected.version != drift.Current {
		return fmt.Errorf("%s pin changed since check", drift.Name)
	}
	path, data, _, err := readCargo(c.Root)
	if err != nil {
		return err
	}
	field := ""
	if selected.inline {
		field = "version"
	}
	updated, err := replaceCargoVersion(data, "workspace.dependencies", drift.Name, field, drift.Current, drift.Latest)
	if err != nil && selected.inline {
		updated, err = replaceCargoVersion(data, "workspace.dependencies."+drift.Name, "version", "", drift.Current, drift.Latest)
	}
	if err != nil {
		return err
	}
	return os.WriteFile(path, updated, 0o644)
}

// FinishFix refreshes Cargo.lock when the workspace has members to resolve.
func (c Crates) FinishFix(ctx context.Context) error {
	_, _, manifest, err := readCargo(c.Root)
	if err != nil {
		return err
	}
	if len(manifest.Workspace.Members) == 0 {
		return nil
	}
	if c.CargoBinary == "" {
		return fmt.Errorf("Cargo executable is not configured")
	}
	cmd := exec.CommandContext(ctx, c.CargoBinary, "update", "--workspace")
	cmd.Dir = c.Root
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("cargo update: %w: %s", err, strings.TrimSpace(string(output)))
	}
	return nil
}

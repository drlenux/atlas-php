"""Validate installation into another project without a daemon or Atlas source tree."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class DistributionContract(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="atlas-consumer-")
        self.addCleanup(self.directory.cleanup)
        self.project = Path(self.directory.name).resolve()
        self.empty_env = self.project / "empty.env"
        self.empty_env.write_text("")
        self.existing = self.project / "compose.yaml"
        self.existing.write_text("""services:
  application-db:
    image: postgres:17
    environment:
      POSTGRES_DB: ${POSTGRES_DB}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
    volumes:
      - application-db:/var/lib/postgresql/data
volumes:
  application-db:
""")
        self.env = {k: v for k, v in os.environ.items()
                    if not k.startswith(("ATLAS_", "POSTGRES_", "PG", "COMPOSE_"))}
        self.env.update({
            "ATLAS_IMAGE": "registry.example.invalid/team/code-atlas:contract",
            "ATLAS_POSTGRES_PASSWORD": "atlas-test-quoted'password#[]",
            "ATLAS_AGENT_TOKEN": "contract-agent",
            "ATLAS_REVIEW_TOKEN": "contract-reviewer",
            "ATLAS_HOST_PORT": "18080",
            "POSTGRES_DB": "unrelated-project",
            "POSTGRES_PASSWORD": "unrelated-password",
            "POSTGRES_USER": "unrelated-user",
        })

    def compose(self, *files, project=None):
        command = ["docker", "compose", "--env-file", str(self.empty_env),
                   "--project-directory", str(project or self.project), "-p", "atlas-contract"]
        for file in files:
            command.extend(["-f", str(file)])
        return subprocess.run(command + ["config", "--format", "json"], env=self.env,
                              text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def model(self, *files, project=None):
        result = self.compose(*files, project=project)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def assert_consumer(self, model):
        atlas = model["services"]["atlas"]
        self.assertNotIn("build", atlas)
        self.assertEqual(atlas["image"], self.env["ATLAS_IMAGE"])
        self.assertNotIn("container_name", atlas)
        self.assertNotIn("depends_on", atlas)
        env = atlas["environment"]
        self.assertEqual(env["POSTGRES_PASSWORD"], self.env["ATLAS_POSTGRES_PASSWORD"])
        self.assertEqual(env["PGPASSWORD"], env["POSTGRES_PASSWORD"])
        self.assertEqual(env["PGUSER"], env["POSTGRES_USER"])
        self.assertEqual(env["PGDATABASE"], env["POSTGRES_DB"])
        self.assertEqual(env["POSTGRES_DB"], "atlas")
        # The consumer must work even when the published image has no ENV defaults.
        self.assertEqual(env.get("ATLAS_PROJECT_ROOT"), "/project")
        self.assertEqual(env.get("ATLAS_BIND"), "0.0.0.0:8080")
        self.assertEqual(env.get("ATLAS_DATABASE_URL"), "host=127.0.0.1 connect_timeout=5")
        self.assertEqual(json.loads(env["ATLAS_ALLOWED_ORIGINS"]),
                         ["http://localhost:18080", "http://127.0.0.1:18080"])
        # Analysis settings live in the project's .atlas/setup.yaml; passing a moved key
        # would make atlas refuse to start next to that file.
        moved = {"ATLAS_WATCH_DIRS", "ATLAS_EXCLUDE", "ATLAS_INCLUDE_VENDOR",
                 "ATLAS_DECLARATION_PATHS", "ATLAS_LANGUAGE_PLUGINS", "ATLAS_FRAMEWORK_PLUGINS",
                 "ATLAS_PHP_VERSION", "ATLAS_MAX_FILE_BYTES", "ATLAS_MAX_ANALYSIS_BYTES",
                 "ATLAS_MAX_GRAPH_ELEMENTS", "ATLAS_ANALYSIS_TIMEOUT_SECS", "ATLAS_DEBOUNCE_MS",
                 "ATLAS_POLL_MS", "ATLAS_GREP_MAX_FILE_BYTES", "ATLAS_TEST_PATHS",
                 "ATLAS_MAX_CONTEXT_NODES", "ATLAS_MAX_CONTEXT_DEPTH", "ATLAS_MAX_REQUEST_BYTES"}
        self.assertEqual(sorted(moved & set(env)), [])
        source = next(v for v in atlas["volumes"] if v["target"] == "/project")
        self.assertEqual(Path(source["source"]), self.project)
        self.assertTrue(source["read_only"])
        self.assertFalse(source.get("bind", {}).get("create_host_path", False))
        database = next(v for v in atlas["volumes"] if v["target"] == "/var/lib/postgresql/data")
        self.assertEqual(database["source"], "atlas-postgres")
        self.assertEqual(model["volumes"]["atlas-postgres"]["name"], "atlas-contract_atlas-postgres")

    def test_full_compose_installs_inside_unrelated_project_without_checkout(self):
        model = self.model(self.existing, ROOT / "docker-compose.yml")
        self.assert_consumer(model)
        self.assertEqual(model["services"]["application-db"]["environment"]["POSTGRES_DB"],
                         "unrelated-project")
        self.assertFalse((self.project / "Dockerfile").exists())

    def test_copyable_example_is_self_contained(self):
        fragment = self.project / "atlas.yaml"
        fragment.write_text((ROOT / "examples/compose.atlas.yml").read_text())
        self.assert_consumer(self.model(self.existing, fragment))

    def test_default_image_uses_owner_registry_and_never_builds(self):
        self.env.pop("ATLAS_IMAGE")
        model = self.model(ROOT / "docker-compose.yml")
        atlas = model["services"]["atlas"]
        self.assertEqual(atlas["image"], "drlenux/memory-tools:0.1.0")
        self.assertNotIn("build", atlas)

    def test_explicit_database_overrides_are_consistent(self):
        self.env.update(ATLAS_POSTGRES_DB="atlas_custom", ATLAS_POSTGRES_USER="atlas_custom_user")
        for file in [ROOT / "docker-compose.yml", ROOT / "examples/compose.atlas.yml"]:
            env = self.model(file)["services"]["atlas"]["environment"]
            self.assertEqual(env["POSTGRES_DB"], env["PGDATABASE"])
            self.assertEqual(env["POSTGRES_DB"], "atlas_custom")
            self.assertEqual(env["POSTGRES_USER"], env["PGUSER"])
            self.assertEqual(env["POSTGRES_USER"], "atlas_custom_user")

    def test_indexing_workers_are_forwarded_in_both_consumer_recipes(self):
        for file in [ROOT / "docker-compose.yml", ROOT / "examples/compose.atlas.yml"]:
            env = self.model(file)["services"]["atlas"]["environment"]
            self.assertEqual(env.get("ATLAS_INDEX_WORKERS"), "0")
        self.env.update(ATLAS_INDEX_WORKERS="4", ATLAS_CPU_LIMIT="4")
        for file in [ROOT / "docker-compose.yml", ROOT / "examples/compose.atlas.yml"]:
            atlas = self.model(file)["services"]["atlas"]
            self.assertEqual(atlas["environment"]["ATLAS_INDEX_WORKERS"], "4")
            self.assertEqual(float(atlas["cpus"]), 4)

    def test_foreign_postgres_password_cannot_configure_atlas(self):
        self.env.pop("ATLAS_POSTGRES_PASSWORD")
        result = self.compose(ROOT / "docker-compose.yml")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ATLAS_POSTGRES_PASSWORD", result.stderr)

    def test_publisher_build_does_not_need_runtime_secrets(self):
        for key in ["ATLAS_POSTGRES_PASSWORD", "ATLAS_AGENT_TOKEN", "ATLAS_REVIEW_TOKEN"]:
            self.env.pop(key)
        model = self.model(ROOT / "docker-compose.build.yml", project=ROOT)
        atlas = model["services"]["atlas"]
        self.assertEqual(atlas["image"], self.env["ATLAS_IMAGE"])
        self.assertEqual(Path(atlas["build"]["context"]), ROOT)
        self.assertNotIn("environment", atlas)
        self.assertNotIn("volumes", atlas)


if __name__ == "__main__":
    unittest.main()

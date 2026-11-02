# frozen_string_literal: true

# native-packages supplies an owned copy, already signed when Apple credentials
# are configured. Keep it unchanged and package only this input.
require "fileutils"
require "tmpdir"
root = File.expand_path("../..", __dir__)
brand = File.read(File.join(root, "brand.toml"))
product_name = brand.match(/^name\s*=\s*"([^"]+)"$/)[1] or abort "brand.toml is missing name"

payload, output = ARGV
abort "usage: dmg.rb PAYLOAD OUTPUT.dmg" unless payload && output
abort "output already exists: #{output}" if File.exist?(output)
Dir.mktmpdir("#{product_name.downcase}-dmg-") do |directory|
  FileUtils.cp_r(File.join(payload, "."), directory, preserve: true)
  app = File.join(directory, "#{product_name}.app")
  abort "expected the signed #{product_name}.app input" unless File.directory?(app)
  File.symlink("/Applications", File.join(directory, "Applications"))
  abort "DMG creation failed" unless system("hdiutil", "create", "-volname", product_name,
    "-srcfolder", directory, "-format", "UDZO", output)
  abort "DMG verification failed" unless system("hdiutil", "verify", output)
end
